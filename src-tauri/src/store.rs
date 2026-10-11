use crate::model::*;
use rusqlite::{params, Connection};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex},
};
use tauri::{AppHandle, Emitter, Manager};
pub type Result<T> = std::result::Result<T, String>;
pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub struct Core {
    pub document: Mutex<Snapshot>,
    pub db: Mutex<Connection>,
    pub root: PathBuf,
    pub cancel: AtomicBool,
    pub file_gate: Mutex<()>,
    pub prepared: Mutex<Option<(PathBuf, String)>>,
}
impl Core {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root).map_err(err)?;
        let root = fs::canonicalize(root).map_err(err)?;
        for name in ["config", "database", "quarantine", "downloads"] {
            fs::create_dir_all(root.join(name)).map_err(err)?;
            private_dir(&root.join(name))?;
        }
        private_dir(&root)?;
        let db = Connection::open(root.join("state.sqlite")).map_err(err)?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS runtime_kv (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY, json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS scans (id TEXT PRIMARY KEY, json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS quarantine (id TEXT PRIMARY KEY, json TEXT NOT NULL);").map_err(err)?;
        let settings = db
            .query_row("SELECT json FROM settings WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut history: Vec<Scan> = read_rows(&db, "scans")?;
        for scan in &mut history {
            if scan.status == "running" {
                scan.status = "interrupted".into();
                scan.finished_at = Some(now());
                scan.message = Some("app_closed".into());
                save_row(&db, "scans", &scan.id, scan)?;
            }
        }
        history.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        let mut quarantine = read_rows(&db, "quarantine")?;
        crate::quarantine::reconcile(&root, &db, &mut quarantine)?;
        Ok(Self {
            document: Mutex::new(Snapshot {
                engine: Engine::default(),
                scan: None,
                history,
                quarantine,
                settings,
                activity: vec![],
                operation: None,
                activity_floating: false,
                activity_dock: "right".into(),
                installer_ready: false,
                data_dir: root.to_string_lossy().into(),
            }),
            db: Mutex::new(db),
            root,
            cancel: AtomicBool::new(false),
            file_gate: Mutex::new(()),
            prepared: Mutex::new(None),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.document.lock().unwrap().clone()
    }
    pub fn publish(&self, app: &AppHandle) {
        let _ = app.emit("core:state", self.snapshot());
    }
    pub fn log(&self, app: &AppHandle, code: &str, detail: impl Into<String>) {
        {
            let mut d = self.document.lock().unwrap();
            d.activity.insert(
                0,
                Activity {
                    at: now(),
                    code: code.into(),
                    detail: detail.into(),
                },
            );
            d.activity.truncate(100);
        }
        self.publish(app);
    }
    pub fn save_scan(&self, scan: &Scan) -> Result<()> {
        save_row(&self.db.lock().unwrap(), "scans", &scan.id, scan)
    }
}
fn read_rows<T: serde::de::DeserializeOwned>(db: &Connection, table: &str) -> Result<Vec<T>> {
    let mut st = db
        .prepare(&format!("SELECT json FROM {table}"))
        .map_err(err)?;
    let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
    rows.map(|r| serde_json::from_str(&r.map_err(err)?).map_err(err))
        .collect()
}
pub fn save_row<T: serde::Serialize>(
    db: &Connection,
    table: &str,
    id: &str,
    value: &T,
) -> Result<()> {
    db.execute(
        &format!("INSERT OR REPLACE INTO {table}(id,json) VALUES (?1,?2)"),
        params![id, serde_json::to_string(value).map_err(err)?],
    )
    .map_err(err)?;
    Ok(())
}
pub fn private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(err)?;
    }
    Ok(())
}
pub fn core(app: &AppHandle) -> tauri::State<'_, Core> {
    app.state::<Core>()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovers_interrupted_scan_and_settings() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        {
            let c = Core::new(root.clone()).unwrap();
            let scan = Scan {
                id: "test".into(),
                kind: "quick".into(),
                target: "/tmp".into(),
                started_at: now(),
                finished_at: None,
                status: "running".into(),
                scanned: 2,
                errors: 0,
                current_file: String::new(),
                findings: vec![],
                message: None,
            };
            c.save_scan(&scan).unwrap();
        }
        let c = Core::new(root.clone()).unwrap();
        assert_eq!(c.snapshot().history[0].status, "interrupted");
        assert_eq!(c.snapshot().history[0].scanned, 2);
        drop(c);
        fs::remove_dir_all(root).unwrap();
    }
}
