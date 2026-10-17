use crate::model::*;
use rusqlite::{params, Connection};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
};
pub type Result<T> = std::result::Result<T, String>;
pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub trait StateNotifier: Send + Sync {
    fn publish(&self, snapshot: Snapshot);
}
struct SilentNotifier;
impl StateNotifier for SilentNotifier {
    fn publish(&self, _snapshot: Snapshot) {}
}
pub struct Core {
    pub document: Mutex<Snapshot>,
    pub db: Mutex<Connection>,
    pub root: PathBuf,
    pub cancel: AtomicBool,
    pub integrity_cancel: AtomicBool,
    pub definitions_cancel: AtomicBool,
    pub file_gate: Mutex<()>,
    pub(crate) settings_gate: Mutex<()>,
    pub prepared: Mutex<Option<(PathBuf, String)>>,
    pub(crate) ipc_active: AtomicBool,
    notifier: Arc<dyn StateNotifier>,
    // Last field: the DB and document are dropped before process ownership is released.
    _ownership: crate::ownership::Ownership,
}
impl Core {
    pub fn new(root: PathBuf) -> Result<Self> {
        Self::with_notifier(root, Arc::new(SilentNotifier))
    }
    pub fn with_notifier(root: PathBuf, notifier: Arc<dyn StateNotifier>) -> Result<Self> {
        fs::create_dir_all(&root).map_err(err)?;
        let root = fs::canonicalize(root).map_err(err)?;
        let ownership = crate::ownership::Ownership::acquire(&root)?;
        private_dir(&root)?;
        for name in ["config", "database", "quarantine", "downloads"] {
            fs::create_dir_all(root.join(name)).map_err(err)?;
            private_dir(&root.join(name))?;
        }
        let db = Connection::open(root.join("state.sqlite")).map_err(err)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS file_transfers (id TEXT PRIMARY KEY, json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS runtime_kv (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY, json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS scans (id TEXT PRIMARY KEY, json TEXT NOT NULL); CREATE TABLE IF NOT EXISTS quarantine (id TEXT PRIMARY KEY, json TEXT NOT NULL);").map_err(err)?;
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
        let pending = crate::file_transfer::recover(&db)?;
        let mut quarantine = read_rows(&db, "quarantine")?;
        crate::quarantine::reconcile(&root, &db, &mut quarantine)?;
        for item in &mut quarantine {
            if pending.contains(&item.id) {
                item.status = "transfer_pending".into();
            }
        }
        for scan in &mut history {
            for finding in &mut scan.findings {
                finding.quarantined = quarantine.iter().any(|q| {
                    q.id == finding.id
                        && matches!(q.status.as_str(), "active" | "review" | "transfer_pending")
                });
            }
            save_row(&db, "scans", &scan.id, scan)?;
        }
        let diagnostics = crate::diagnostics::load(&db)?;
        Ok(Self {
            document: Mutex::new(Snapshot {
                diagnostics,
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
            integrity_cancel: AtomicBool::new(false),
            definitions_cancel: AtomicBool::new(false),
            file_gate: Mutex::new(()),
            settings_gate: Mutex::new(()),
            prepared: Mutex::new(None),
            ipc_active: AtomicBool::new(false),
            notifier,
            _ownership: ownership,
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.document.lock().unwrap().clone()
    }
    pub fn publish(&self) {
        self.notifier.publish(self.snapshot());
    }
    pub fn log(&self, code: &str, detail: impl Into<String>) {
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
        self.publish();
    }
    pub fn save_scan(&self, scan: &Scan) -> Result<()> {
        save_row(&self.db.lock().unwrap(), "scans", &scan.id, scan)
    }
}
pub(crate) fn read_rows<T: serde::de::DeserializeOwned>(
    db: &Connection,
    table: &str,
) -> Result<Vec<T>> {
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
