//! Bounded evidence of worker readiness, not a claim about why macOS started it.
use crate::store::{now, Core, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const KEY: &str = "worker_starts_v1";
const LIMIT: usize = 16;
const MAX_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Start {
    pub instance_id: String,
    pub pid: u32,
    pub parent_pid_at_ready: Option<u32>,
    pub ready_at: String,
    pub app_version: String,
    pub journal_persisted: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub current: Option<Start>,
    pub recent: Option<Vec<Start>>,
}

fn records(db: &Connection) -> Result<Vec<Start>> {
    let size: Option<usize> = db
        .query_row(
            "SELECT length(CAST(value AS BLOB)) FROM runtime_kv WHERE key=?1",
            [KEY],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| "worker_journal_read_failed")?;
    match size {
        None => return Ok(vec![]),
        Some(size) if size > MAX_BYTES => return Err("worker_journal_invalid".into()),
        _ => (),
    }
    let json: String = db
        .query_row("SELECT value FROM runtime_kv WHERE key=?1", [KEY], |r| {
            r.get(0)
        })
        .map_err(|_| "worker_journal_read_failed")?;
    let records: Vec<Start> = serde_json::from_str(&json).map_err(|_| "worker_journal_invalid")?;
    if records.len() > LIMIT
        || records.iter().any(|r| {
            uuid::Uuid::parse_str(&r.instance_id).is_err()
                || r.pid == 0
                || chrono::DateTime::parse_from_rfc3339(&r.ready_at).is_err()
                || r.app_version.len() > 64
                || !r.journal_persisted
        })
    {
        return Err("worker_journal_invalid".into());
    }
    Ok(records)
}

fn append(db: &Connection, start: &Start) -> Result<()> {
    let mut recent = records(db)?;
    recent.insert(0, start.clone());
    recent.truncate(LIMIT);
    let json = serde_json::to_string(&recent).map_err(|_| "worker_journal_write_failed")?;
    if json.len() > MAX_BYTES {
        return Err("worker_journal_write_failed".into());
    }
    db.execute("INSERT INTO runtime_kv(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![KEY, json])
        .map_err(|_| "worker_journal_write_failed")?;
    Ok(())
}

/// Called once by the worker entry point, after the private IPC socket is bound.
/// A diagnostic write failure must not disable scanning or destroy prior evidence.
pub(crate) fn record_ready(core: &Core) {
    let mut start = Start {
        instance_id: uuid::Uuid::new_v4().to_string(),
        pid: std::process::id(),
        parent_pid_at_ready: parent_pid(),
        ready_at: now(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        journal_persisted: true,
    };
    if append(&core.db.lock().unwrap(), &start).is_err() {
        start.journal_persisted = false;
    }
    *core.worker_start.lock().unwrap() = Some(start);
}

fn parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        u32::try_from(unsafe { libc::getppid() }).ok()
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub(crate) fn report(core: &Core, errors: &mut Vec<String>) -> Report {
    let current = core.worker_start.lock().unwrap().clone();
    if current.as_ref().is_some_and(|s| !s.journal_persisted) {
        errors.push("worker_journal_write_failed".into());
    }
    let recent = match records(&core.db.lock().unwrap()) {
        Ok(records) => Some(records),
        Err(reason) => {
            errors.push(reason);
            None
        }
    };
    Report { current, recent }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (Fixture, Core) {
        let f =
            Fixture(std::env::temp_dir().join(format!("worker-journal-{}", uuid::Uuid::new_v4())));
        let core = Core::new(f.0.clone()).unwrap();
        (f, core)
    }
    #[test]
    fn bounds_history_and_does_not_misidentify_a_reopened_gui_as_a_worker() {
        let (f, core) = fixture();
        assert!(report(&core, &mut vec![]).current.is_none());
        let mut first = String::new();
        for n in 0..LIMIT + 2 {
            record_ready(&core);
            let start = core.worker_start.lock().unwrap().clone().unwrap();
            assert_eq!(start.pid, std::process::id());
            assert!(start.journal_persisted);
            if n == 0 {
                first = start.instance_id;
            }
        }
        let history = records(&core.db.lock().unwrap()).unwrap();
        assert_eq!(history.len(), LIMIT);
        assert!(!history.iter().any(|s| s.instance_id == first));
        drop(core);
        let reopened = Core::new(f.0.clone()).unwrap();
        let result = report(&reopened, &mut vec![]);
        assert!(result.current.is_none());
        assert_eq!(result.recent.unwrap(), history);
    }
    #[test]
    fn bad_or_unwritable_journal_preserves_evidence_and_returns_partial_diagnostics() {
        let (_f, core) = fixture();
        for bad in ["bad-json".to_string(), "x".repeat(MAX_BYTES + 1)] {
            core.db
                .lock()
                .unwrap()
                .execute(
                    "INSERT OR REPLACE INTO runtime_kv VALUES (?1,?2)",
                    params![KEY, bad],
                )
                .unwrap();
            record_ready(&core);
            let diagnostic = crate::diagnostics::report(&core).unwrap();
            assert!(!diagnostic.worker_runtime.current.unwrap().journal_persisted);
            assert!(diagnostic.worker_runtime.recent.is_none());
            assert!(diagnostic
                .collection_errors
                .iter()
                .any(|e| e == "worker_journal_invalid"));
            let retained: String = core
                .db
                .lock()
                .unwrap()
                .query_row("SELECT value FROM runtime_kv WHERE key=?1", [KEY], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(retained, bad);
        }
        core.db
            .lock()
            .unwrap()
            .execute("DELETE FROM runtime_kv WHERE key=?1", [KEY])
            .unwrap();
        record_ready(&core);
        let history = records(&core.db.lock().unwrap()).unwrap();
        core.db
            .lock()
            .unwrap()
            .execute_batch("PRAGMA query_only=ON")
            .unwrap();
        record_ready(&core);
        let diagnostic = crate::diagnostics::report(&core).unwrap();
        assert!(!diagnostic.worker_runtime.current.unwrap().journal_persisted);
        assert!(diagnostic
            .collection_errors
            .iter()
            .any(|e| e == "worker_journal_write_failed"));
        assert_eq!(records(&core.db.lock().unwrap()).unwrap(), history);
    }
}
