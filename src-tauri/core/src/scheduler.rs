//! Daily claims are durable and shared by GUI and worker owners. No OS service
//! registration, wake-from-sleep, catch-up scan, or automatic quarantine here.
use crate::{
    engine,
    model::{Scan, Settings, Snapshot},
    scanner,
    store::{err, Core, Result},
};
use chrono::{DateTime, Duration, Local, LocalResult, NaiveDate, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub local_date: String,
    pub kind: String,
    pub local_hour: u32,
    pub timezone: String,
    pub settings_revision: String,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub deadline: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: String,
    pub reason: Option<String>,
    pub detail: Option<String>,
    pub scan_id: Option<String>,
}
#[derive(Clone)]
pub struct Moment {
    pub now: DateTime<Utc>,
    pub date: NaiveDate,
    pub scheduled: Option<DateTime<Utc>>,
    pub past_local_hour: bool,
    pub timezone: String,
}
impl Moment {
    pub fn at<T: TimeZone>(now: DateTime<T>, hour: u32, timezone: String) -> Self {
        let local = now
            .date_naive()
            .and_hms_opt(hour, 0, 0)
            .expect("validated schedule hour");
        let scheduled = match now.timezone().from_local_datetime(&local) {
            LocalResult::Single(at) => Some(at.with_timezone(&Utc)),
            // A repeated wall-clock hour always uses its FIRST occurrence.
            LocalResult::Ambiguous(a, b) => Some(a.min(b).with_timezone(&Utc)),
            LocalResult::None => None,
        };
        Self {
            now: now.with_timezone(&Utc),
            date: now.date_naive(),
            scheduled,
            past_local_hour: now.naive_local() >= local,
            timezone,
        }
    }
}
pub trait Clock {
    fn moment(&self, hour: u32) -> Moment;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn moment(&self, hour: u32) -> Moment {
        let now = Local::now();
        Moment::at(now, hour, format!("local ({})", now.offset()))
    }
}
#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    None,
    Scan { date: String, id: String },
    Update { at: DateTime<Utc> },
}
fn revision(settings: &Settings) -> Result<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(settings).map_err(err)?,
    )))
}
fn write(db: &Connection, run: &Run) -> Result<()> {
    db.execute("INSERT INTO scheduled_runs(local_date,kind,status,scan_id,json) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(local_date,kind) DO UPDATE SET status=excluded.status,scan_id=excluded.scan_id,json=excluded.json",
        params![run.local_date,run.kind,run.status,run.scan_id,serde_json::to_string(run).map_err(err)?]).map_err(err)?;
    Ok(())
}
fn get(db: &Connection, date: &str) -> Result<Option<Run>> {
    db.query_row(
        "SELECT json FROM scheduled_runs WHERE local_date=?1 AND kind='quick'",
        [date],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(err)?
    .map(|json| serde_json::from_str(&json).map_err(err))
    .transpose()
}
pub fn records(db: &Connection) -> Result<Vec<Run>> {
    let mut stmt = db
        .prepare("SELECT json FROM scheduled_runs ORDER BY local_date DESC,kind")
        .map_err(err)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
    rows.map(|row| serde_json::from_str(&row.map_err(err)?).map_err(err))
        .collect()
}
pub fn latest(db: &Connection) -> Result<Option<Run>> {
    db.query_row(
        "SELECT json FROM scheduled_runs ORDER BY local_date DESC LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(err)?
    .map(|json| serde_json::from_str(&json).map_err(err))
    .transpose()
}
/// Called only after process ownership and normal scan recovery are established.
pub(crate) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS scheduled_runs(local_date TEXT NOT NULL,kind TEXT NOT NULL,status TEXT NOT NULL,scan_id TEXT,json TEXT NOT NULL,PRIMARY KEY(local_date,kind));").map_err(err)?;
    // Preserve the old GUI daily claim across this migration and later handoffs.
    let legacy: Option<String> = db
        .query_row(
            "SELECT value FROM runtime_kv WHERE key='last_scheduled_scan'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    if let Some(date) = legacy.filter(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok()) {
        if get(db, &date)?.is_none() {
            write(
                db,
                &Run {
                    local_date: date,
                    kind: "quick".into(),
                    local_hour: 0,
                    timezone: "legacy".into(),
                    settings_revision: "legacy".into(),
                    scheduled_at: None,
                    deadline: None,
                    created_at: Utc::now(),
                    started_at: None,
                    finished_at: None,
                    status: "interrupted".into(),
                    reason: Some("legacy_claim".into()),
                    detail: None,
                    scan_id: None,
                },
            )?;
        }
    }
    for mut run in records(db)?.into_iter().filter(|r| r.status == "running") {
        let scan: Option<Scan> = if let Some(id) = &run.scan_id {
            db.query_row("SELECT json FROM scans WHERE id=?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(err)?
            .map(|json| serde_json::from_str(&json).map_err(err))
            .transpose()?
        } else {
            None
        };
        if let Some(scan) = scan.filter(|s| s.status != "running") {
            finish(&mut run, &scan);
        } else {
            run.status = "interrupted".into();
            run.reason = Some("owner_restarted".into());
            run.finished_at = Some(Utc::now());
        }
        write(db, &run)?;
    }
    Ok(())
}
fn finish(run: &mut Run, scan: &Scan) {
    run.status = scan.status.clone();
    run.started_at = DateTime::parse_from_rfc3339(&scan.started_at)
        .ok()
        .map(|t| t.with_timezone(&Utc));
    run.finished_at = scan
        .finished_at
        .as_ref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc));
    run.reason = match scan.status.as_str() {
        "completed" => None,
        "cancelled" => Some("scan_cancelled".into()),
        "interrupted" => Some("owner_restarted".into()),
        "incomplete" => Some("scan_incomplete".into()),
        _ => Some("scan_failed".into()),
    };
    run.detail = scan.message.clone();
}
/// Save a scheduled outcome in the SAME transaction as its scan history.
pub(crate) fn scan_saved(db: &Connection, scan: &Scan) -> Result<()> {
    let json: Option<String> = db
        .query_row(
            "SELECT json FROM scheduled_runs WHERE scan_id=?1 AND status='running'",
            [&scan.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    if let Some(json) = json {
        let mut run: Run = serde_json::from_str(&json).map_err(err)?;
        if scan.status == "running" {
            run.started_at = DateTime::parse_from_rfc3339(&scan.started_at)
                .ok()
                .map(|t| t.with_timezone(&Utc));
        } else {
            finish(&mut run, scan);
        }
        write(db, &run)?;
    }
    Ok(())
}
pub(crate) fn publish(core: &Core) -> Result<()> {
    let latest = latest(&core.db.lock().unwrap())?;
    let changed = {
        let mut doc = core.document.lock().unwrap();
        if doc.scheduled_run == latest {
            false
        } else {
            doc.scheduled_run = latest;
            true
        }
    };
    if changed {
        core.publish();
    }
    Ok(())
}
fn busy(snapshot: &Snapshot) -> bool {
    snapshot.operation.is_some()
        || snapshot
            .scan
            .as_ref()
            .is_some_and(|s| s.status == "running")
}
fn cooldown(snapshot: &Snapshot, at: DateTime<Utc>) -> bool {
    snapshot.diagnostics.iter().any(|a| {
        a.kind == "definitions_update"
            && a.status == "failed"
            && a.code == "rate_limit"
            && a.retry_not_before
                .as_ref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .is_some_and(|t| t > at)
    })
}
fn update_due(db: &Connection, at: DateTime<Utc>) -> Result<bool> {
    let previous: Option<String> = db
        .query_row(
            "SELECT value FROM runtime_kv WHERE key='last_update_attempt'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    Ok(previous
        .and_then(|s| s.parse::<i64>().ok())
        .is_none_or(|t| at.timestamp().saturating_sub(t) >= 6 * 3600))
}
fn blocked(snapshot: &Snapshot, at: DateTime<Utc>) -> Option<&'static str> {
    if busy(snapshot) {
        Some("busy")
    } else if !snapshot.engine.checked {
        Some("engine_checking")
    } else if snapshot.engine.version.is_none() {
        Some("engine_missing")
    } else if !snapshot.engine.definitions {
        Some(if cooldown(snapshot, at) {
            "update_cooldown"
        } else {
            "definitions_missing"
        })
    } else {
        None
    }
}
pub(crate) fn plan(core: &Core, clock: &impl Clock) -> Result<Action> {
    let _settings = core.settings_gate.lock().unwrap();
    let snapshot = core.snapshot();
    let moment = clock.moment(snapshot.settings.schedule_hour);
    let today = moment.date.to_string();
    let mut db = core.db.lock().unwrap();
    let tx = db.transaction().map_err(err)?;
    // Only dates already observed while running have records. Never synthesize
    // jobs for unobserved days during power-off, nor run an old pending job.
    for mut old in records(&tx)?
        .into_iter()
        .filter(|r| r.local_date < today && matches!(r.status.as_str(), "pending" | "disabled"))
    {
        old.status = if old.status == "disabled" {
            "cancelled"
        } else {
            "missed"
        }
        .into();
        old.reason = Some(old.reason.unwrap_or_else(|| "window_expired".into()));
        old.finished_at = Some(moment.now);
        write(&tx, &old)?;
    }
    let previous = get(&tx, &today)?;
    let mut action = Action::None;
    if snapshot.settings.schedule_enabled || previous.is_some() {
        let mut run = previous.clone().unwrap_or_else(|| Run {
            local_date: today.clone(),
            kind: "quick".into(),
            local_hour: snapshot.settings.schedule_hour,
            timezone: moment.timezone.clone(),
            settings_revision: String::new(),
            scheduled_at: moment.scheduled,
            deadline: moment.scheduled.map(|t| t + Duration::hours(1)),
            created_at: moment.now,
            started_at: None,
            finished_at: None,
            status: "pending".into(),
            reason: None,
            detail: None,
            scan_id: None,
        });
        if matches!(run.status.as_str(), "pending" | "disabled") {
            if !snapshot.settings.schedule_enabled {
                run.status = "disabled".into();
                run.reason = Some("schedule_disabled".into());
            } else {
                run.status = "pending".into();
                run.local_hour = snapshot.settings.schedule_hour;
                run.timezone = moment.timezone.clone();
                run.settings_revision = revision(&snapshot.settings)?;
                run.scheduled_at = moment.scheduled;
                run.deadline = moment.scheduled.map(|t| t + Duration::hours(1));
                if moment.scheduled.is_none() && moment.past_local_hour {
                    run.status = "missed".into();
                    run.reason = Some("local_time_missing".into());
                    run.finished_at = Some(moment.now);
                } else if run.deadline.is_some_and(|t| moment.now >= t) {
                    run.status = "missed".into();
                    run.reason = Some(
                        run.reason
                            .filter(|r| r != "schedule_disabled")
                            .unwrap_or_else(|| "window_expired".into()),
                    );
                    run.finished_at = Some(moment.now);
                } else if moment.scheduled.is_some_and(|t| moment.now >= t) {
                    run.reason = blocked(&snapshot, moment.now).map(str::to_owned);
                    if run.reason.is_none() {
                        let id = uuid::Uuid::new_v4().to_string();
                        run.status = "running".into();
                        run.scan_id = Some(id.clone());
                        // Persist before any scan child exists. A crash here is
                        // interrupted, never a reason to replay the scan.
                        action = Action::Scan {
                            date: today.clone(),
                            id,
                        };
                    }
                } else {
                    run.reason = None;
                }
            }
        }
        if previous.as_ref() != Some(&run) {
            write(&tx, &run)?;
        }
    }
    // A due, ready scan has priority over the six-hour update. Missing definitions
    // may instead be supplied by an automatic update inside the scan window.
    if action == Action::None
        && snapshot.settings.auto_update
        && snapshot.engine.checked
        && snapshot.engine.version.is_some()
        && !busy(&snapshot)
        && !cooldown(&snapshot, moment.now)
        && update_due(&tx, moment.now)?
    {
        action = Action::Update { at: moment.now };
    }
    tx.commit().map_err(err)?;
    drop(db);
    drop(_settings);
    publish(core)?;
    Ok(action)
}
pub(crate) fn start_claimed(
    core: &Arc<Core>,
    date: &str,
    id: &str,
    clock: &impl Clock,
) -> Result<()> {
    let _settings = core.settings_gate.lock().unwrap();
    let snapshot = core.snapshot();
    let run = get(&core.db.lock().unwrap(), date)?.ok_or("schedule_claim_missing")?;
    if run.status != "running" || run.scan_id.as_deref() != Some(id) {
        return Err("schedule_claim_changed".into());
    }
    let moment = clock.moment(snapshot.settings.schedule_hour);
    // A clock rollback or timezone/settings change can invalidate a claim
    // before its child starts. Re-plan it using the current wall clock.
    if moment.scheduled != run.scheduled_at || run.scheduled_at.is_some_and(|at| moment.now < at) {
        let mut pending = run;
        pending.status = "pending".into();
        pending.scan_id = None;
        pending.reason = None;
        write(&core.db.lock().unwrap(), &pending)?;
        drop(_settings);
        return publish(core);
    }
    if moment.date.to_string() != date
        || run.deadline.is_some_and(|deadline| moment.now >= deadline)
    {
        let mut expired = run;
        expired.status = "missed".into();
        expired.reason = Some("window_expired".into());
        expired.finished_at = Some(moment.now);
        write(&core.db.lock().unwrap(), &expired)?;
        drop(_settings);
        return publish(core);
    }
    let result = if !snapshot.settings.schedule_enabled
        || revision(&snapshot.settings)? != run.settings_revision
    {
        Err("schedule_changed".into())
    } else {
        scanner::start_scheduled(core.clone(), id.to_owned())
    };
    if let Err(reason) = result {
        let mut db = core.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        let mut run = get(&tx, date)?.ok_or("schedule_claim_missing")?;
        if matches!(
            reason.as_str(),
            "busy" | "schedule_changed" | "engine_not_ready"
        ) {
            run.status = "pending".into();
            run.scan_id = None;
            run.reason = Some(
                if reason == "schedule_changed" {
                    "settings_changed"
                } else {
                    "busy"
                }
                .into(),
            );
        } else {
            run.status = "failed".into();
            run.reason = Some(
                if crate::diagnostics::classify(&reason) == "access_denied" {
                    "access_denied"
                } else {
                    "scan_failed"
                }
                .into(),
            );
            run.detail = Some(reason);
            run.finished_at = Some(moment.now);
        }
        write(&tx, &run)?;
        tx.commit().map_err(err)?;
    }
    drop(_settings);
    publish(core)
}
/// Reserve the same operation used by manual updates; persist throttling only
/// after successful reservation. A failed DB write releases it before launch.
pub(crate) fn begin_auto_update(core: &Core, at: DateTime<Utc>) -> Result<bool> {
    let _settings = core.settings_gate.lock().unwrap();
    let snapshot = core.snapshot();
    if !snapshot.settings.auto_update
        || busy(&snapshot)
        || cooldown(&snapshot, at)
        || !snapshot.engine.checked
        || snapshot.engine.version.is_none()
        || !update_due(&core.db.lock().unwrap(), at)?
    {
        return Ok(false);
    }
    engine::begin_update(core)?;
    let saved = core
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO runtime_kv(key,value) VALUES ('last_update_attempt',?1)",
            [at.timestamp().to_string()],
        )
        .map_err(err);
    if let Err(reason) = saved {
        core.document.lock().unwrap().operation = None;
        core.publish();
        return Err(reason);
    }
    Ok(true)
}
pub async fn tick(core: &Arc<Core>, clock: &impl Clock) -> Result<()> {
    let _gate = core.maintenance_gate.try_lock().map_err(|_| "busy")?;
    match plan(core, clock)? {
        Action::None => Ok(()),
        Action::Scan { date, id } => start_claimed(core, &date, &id, clock),
        Action::Update { at } => {
            if begin_auto_update(core, at)? {
                engine::complete_update(core).await?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDateTime};
    struct Fixture {
        core: Arc<Core>,
        root: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::path::PathBuf::from("/private/tmp")
                .join(format!("clamav-schedule-{}", uuid::Uuid::new_v4()));
            let core = Arc::new(Core::new(root.clone()).unwrap());
            {
                let mut doc = core.document.lock().unwrap();
                doc.settings.schedule_enabled = true;
                doc.settings.schedule_hour = 12;
                doc.engine.checked = true;
                doc.engine.version = Some("fixture".into());
                doc.engine.definitions = true;
            }
            Self { core, root }
        }
        fn latest(&self) -> Run {
            latest(&self.core.db.lock().unwrap()).unwrap().unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    struct FixedClock(DateTime<FixedOffset>);
    impl Clock for FixedClock {
        fn moment(&self, hour: u32) -> Moment {
            Moment::at(self.0, hour, "fixture +09:00".into())
        }
    }
    fn clock(text: &str) -> FixedClock {
        FixedClock(DateTime::parse_from_rfc3339(text).unwrap())
    }
    fn claim(f: &Fixture, at: &str) -> (String, String) {
        match plan(&f.core, &clock(at)).unwrap() {
            Action::Scan { date, id } => (date, id),
            other => panic!("expected scan: {other:?}"),
        }
    }
    #[test]
    fn daily_claim_survives_clock_changes_and_settings_changes() {
        let f = Fixture::new();
        assert_eq!(
            plan(&f.core, &clock("2026-10-25T11:59:59+09:00")).unwrap(),
            Action::None
        );
        assert_eq!(f.latest().status, "pending");
        let (_, id) = claim(&f, "2026-10-25T12:00:00+09:00");
        assert_eq!(
            plan(&f.core, &clock("2026-10-25T12:30:00+09:00")).unwrap(),
            Action::None
        );
        f.core.document.lock().unwrap().settings.schedule_hour = 14;
        assert_eq!(
            plan(&f.core, &clock("2026-10-25T14:00:00+09:00")).unwrap(),
            Action::None
        );
        assert_eq!(f.latest().scan_id.as_deref(), Some(id.as_str()));
        assert_eq!(
            plan(&f.core, &clock("2026-10-24T23:00:00+09:00")).unwrap(),
            Action::None
        );
        assert_eq!(
            plan(&f.core, &clock("2026-10-25T14:00:00+09:00")).unwrap(),
            Action::None
        );
    }
    #[test]
    fn reopened_core_recovers_an_unlaunched_claim_and_never_replays_it() {
        let root = std::path::PathBuf::from("/private/tmp")
            .join(format!("clamav-schedule-reopen-{}", uuid::Uuid::new_v4()));
        let core = Core::new(root.clone()).unwrap();
        crate::settings::save(
            &core,
            Settings {
                schedule_enabled: true,
                schedule_hour: 12,
                ..Default::default()
            },
        )
        .unwrap();
        {
            let mut doc = core.document.lock().unwrap();
            doc.engine.checked = true;
            doc.engine.version = Some("fixture".into());
            doc.engine.definitions = true;
        }
        assert!(matches!(
            plan(&core, &clock("2026-10-25T12:00:00+09:00")).unwrap(),
            Action::Scan { .. }
        ));
        drop(core);
        let core = Core::new(root.clone()).unwrap();
        let run = latest(&core.db.lock().unwrap()).unwrap().unwrap();
        assert_eq!(run.status, "interrupted");
        assert_eq!(run.reason.as_deref(), Some("owner_restarted"));
        assert_eq!(
            plan(&core, &clock("2026-10-25T12:01:00+09:00")).unwrap(),
            Action::None
        );
        drop(core);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn busy_waits_only_inside_window_and_new_day_does_not_catch_up() {
        let f = Fixture::new();
        f.core.document.lock().unwrap().operation = Some(Default::default());
        plan(&f.core, &clock("2026-10-25T12:00:00+09:00")).unwrap();
        assert_eq!(f.latest().reason.as_deref(), Some("busy"));
        plan(&f.core, &clock("2026-10-25T13:00:00+09:00")).unwrap();
        assert_eq!(f.latest().status, "missed");
        f.core.document.lock().unwrap().operation = None;
        assert_eq!(
            plan(&f.core, &clock("2026-10-25T12:30:00+09:00")).unwrap(),
            Action::None
        );
        plan(&f.core, &clock("2026-10-27T11:00:00+09:00")).unwrap();
        let rows = records(&f.core.db.lock().unwrap()).unwrap();
        assert_eq!(rows.len(), 2); // no synthetic 26th October job
        assert_eq!(rows[0].status, "pending");
    }
    #[test]
    fn missing_prerequisites_are_distinct_and_scan_has_priority_over_update() {
        let f = Fixture::new();
        let now = clock("2026-10-25T12:15:00+09:00");
        {
            let mut d = f.core.document.lock().unwrap();
            d.engine.version = None;
        }
        plan(&f.core, &now).unwrap();
        assert_eq!(f.latest().reason.as_deref(), Some("engine_missing"));
        {
            let mut d = f.core.document.lock().unwrap();
            d.engine.version = Some("fixture".into());
            d.engine.definitions = false;
        }
        plan(&f.core, &now).unwrap();
        assert_eq!(f.latest().reason.as_deref(), Some("definitions_missing"));
        {
            let mut d = f.core.document.lock().unwrap();
            d.settings.auto_update = true;
        }
        assert!(matches!(
            plan(&f.core, &now).unwrap(),
            Action::Update { .. }
        ));
        f.core
            .document
            .lock()
            .unwrap()
            .diagnostics
            .push(crate::diagnostics::Attempt {
                at: "fixture".into(),
                kind: "definitions_update".into(),
                status: "failed".into(),
                code: "rate_limit".into(),
                detail: None,
                retry_not_before: Some("2099-01-01T00:00:00Z".into()),
            });
        assert_eq!(plan(&f.core, &now).unwrap(), Action::None);
        assert_eq!(f.latest().reason.as_deref(), Some("update_cooldown"));
        f.core.document.lock().unwrap().engine.definitions = true;
        assert!(matches!(plan(&f.core, &now).unwrap(), Action::Scan { .. })); // usable definitions still permit scan
    }
    #[test]
    fn completion_and_history_commit_together_and_failure_rolls_back_both() {
        let f = Fixture::new();
        let (_, id) = claim(&f, "2026-10-25T12:00:00+09:00");
        let scan = Scan {
            id: id.clone(),
            kind: "quick".into(),
            target: "/fixture".into(),
            started_at: "2026-10-25T03:00:00Z".into(),
            finished_at: Some("2026-10-25T03:01:00Z".into()),
            status: "completed".into(),
            scanned: 1,
            errors: 0,
            current_file: String::new(),
            findings: vec![],
            message: None,
        };
        f.core.db.lock().unwrap().execute_batch("CREATE TRIGGER schedule_failure BEFORE UPDATE ON scheduled_runs BEGIN SELECT RAISE(FAIL,'fixture_write_failure'); END;").unwrap();
        assert!(f
            .core
            .save_scan(&scan)
            .unwrap_err()
            .contains("fixture_write_failure"));
        let count: i64 = f
            .core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM scans WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(f.latest().status, "running");
        f.core
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER schedule_failure")
            .unwrap();
        f.core.save_scan(&scan).unwrap();
        assert_eq!(f.latest().status, "completed");
        initialize(&f.core.db.lock().unwrap()).unwrap();
        assert_eq!(f.latest().status, "completed");
    }
    #[test]
    fn legacy_daily_claim_and_pending_disable_are_preserved() {
        let f = Fixture::new();
        f.core
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO runtime_kv(key,value) VALUES ('last_scheduled_scan','2026-10-25')",
                [],
            )
            .unwrap();
        initialize(&f.core.db.lock().unwrap()).unwrap();
        assert_eq!(
            plan(&f.core, &clock("2026-10-25T12:00:00+09:00")).unwrap(),
            Action::None
        );
        assert_eq!(f.latest().reason.as_deref(), Some("legacy_claim"));
        plan(&f.core, &clock("2026-10-26T11:00:00+09:00")).unwrap();
        f.core.document.lock().unwrap().settings.schedule_enabled = false;
        plan(&f.core, &clock("2026-10-26T12:00:00+09:00")).unwrap();
        assert_eq!(f.latest().status, "disabled");
        f.core.document.lock().unwrap().settings.schedule_enabled = true;
        claim(&f, "2026-10-26T12:20:00+09:00");
    }
    #[test]
    fn expired_claim_and_changed_settings_never_launch_a_scan() {
        let f = Fixture::new();
        let (date, id) = claim(&f, "2026-10-25T12:59:59+09:00");
        start_claimed(&f.core, &date, &id, &clock("2026-10-25T13:00:00+09:00")).unwrap();
        assert_eq!(f.latest().status, "missed");
        assert!(f.core.snapshot().scan.is_none());
        let (date, id) = claim(&f, "2026-10-26T12:00:00+09:00");
        f.core.document.lock().unwrap().settings.schedule_enabled = false;
        start_claimed(&f.core, &date, &id, &clock("2026-10-26T12:00:01+09:00")).unwrap();
        assert_eq!(f.latest().status, "pending");
        assert!(f.core.snapshot().scan.is_none());
    }
    #[test]
    fn clock_change_between_claim_and_launch_replans_without_starting() {
        let f = Fixture::new();
        let (date, id) = claim(&f, "2026-10-25T12:00:00+09:00");
        start_claimed(&f.core, &date, &id, &clock("2026-10-25T11:59:59+09:00")).unwrap();
        assert_eq!(f.latest().status, "pending");
        assert!(f.latest().scan_id.is_none());
        assert!(f.core.snapshot().scan.is_none());
        let (date, id) = claim(&f, "2026-10-25T12:00:00+09:00");
        start_claimed(&f.core, &date, &id, &clock("2026-10-25T12:00:01+08:00")).unwrap();
        assert_eq!(f.latest().status, "pending");
        assert!(f.core.snapshot().scan.is_none());
        assert!(matches!(
            plan(&f.core, &clock("2026-10-25T12:00:02+08:00")).unwrap(),
            Action::Scan { .. }
        ));
    }
    #[test]
    fn update_reservation_respects_cooldown_busy_persistence_and_failed_write() {
        let f = Fixture::new();
        f.core.document.lock().unwrap().settings.auto_update = true;
        let now = Utc::now();
        f.core.document.lock().unwrap().operation = Some(Default::default());
        assert!(!begin_auto_update(&f.core, now).unwrap());
        assert!(update_due(&f.core.db.lock().unwrap(), now).unwrap());
        f.core.document.lock().unwrap().operation = None;
        f.core.db.lock().unwrap().execute_batch("CREATE TRIGGER update_failure BEFORE INSERT ON runtime_kv WHEN NEW.key='last_update_attempt' BEGIN SELECT RAISE(FAIL,'fixture_write_failure'); END;").unwrap();
        assert!(begin_auto_update(&f.core, now).is_err());
        assert!(f.core.snapshot().operation.is_none());
        f.core
            .db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER update_failure")
            .unwrap();
        assert!(begin_auto_update(&f.core, now).unwrap());
        assert_eq!(f.core.snapshot().operation.unwrap().kind, "definitions");
        f.core.document.lock().unwrap().operation = None;
        assert!(!begin_auto_update(&f.core, now + Duration::hours(5)).unwrap());
        assert!(!begin_auto_update(&f.core, now - Duration::hours(1)).unwrap());
        assert!(begin_auto_update(&f.core, now + Duration::hours(6)).unwrap());
    }
    // A deterministic timezone with the real 2026 US DST transitions. No process-
    // global TZ mutation, network, or dependency on the host's local timezone.
    #[derive(Clone)]
    struct TestZone;
    impl TimeZone for TestZone {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self {
            Self
        }
        fn offset_from_local_date(&self, date: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, t: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let spring = NaiveDate::from_ymd_opt(2026, 3, 8)
                .unwrap()
                .and_hms_opt(2, 0, 0)
                .unwrap();
            let fall = NaiveDate::from_ymd_opt(2026, 11, 1)
                .unwrap()
                .and_hms_opt(1, 0, 0)
                .unwrap();
            let standard = FixedOffset::west_opt(5 * 3600).unwrap();
            let summer = FixedOffset::west_opt(4 * 3600).unwrap();
            if *t >= spring && *t < spring + Duration::hours(1) {
                LocalResult::None
            } else if *t >= fall && *t < fall + Duration::hours(1) {
                LocalResult::Ambiguous(summer, standard)
            } else {
                LocalResult::Single(if *t >= spring && *t < fall {
                    summer
                } else {
                    standard
                })
            }
        }
        fn offset_from_utc_date(&self, date: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, t: &NaiveDateTime) -> FixedOffset {
            let spring = NaiveDate::from_ymd_opt(2026, 3, 8)
                .unwrap()
                .and_hms_opt(7, 0, 0)
                .unwrap();
            let fall = NaiveDate::from_ymd_opt(2026, 11, 1)
                .unwrap()
                .and_hms_opt(6, 0, 0)
                .unwrap();
            FixedOffset::west_opt(if *t >= spring && *t < fall {
                4 * 3600
            } else {
                5 * 3600
            })
            .unwrap()
        }
    }
    struct ZoneClock(DateTime<Utc>);
    impl Clock for ZoneClock {
        fn moment(&self, hour: u32) -> Moment {
            Moment::at(
                self.0.with_timezone(&TestZone),
                hour,
                "America/New_York fixture".into(),
            )
        }
    }
    #[test]
    fn dst_gap_is_missed_and_repeated_hour_cannot_extend_or_duplicate_window() {
        let f = Fixture::new();
        f.core.document.lock().unwrap().settings.schedule_hour = 2;
        let now = ZoneClock("2026-03-08T07:00:00Z".parse().unwrap());
        plan(&f.core, &now).unwrap();
        assert_eq!(f.latest().reason.as_deref(), Some("local_time_missing"));
        f.core.document.lock().unwrap().settings.schedule_hour = 1;
        let first = ZoneClock("2026-11-01T05:00:00Z".parse().unwrap());
        assert!(matches!(
            plan(&f.core, &first).unwrap(),
            Action::Scan { .. }
        ));
        let second = ZoneClock("2026-11-01T06:00:00Z".parse().unwrap());
        assert_eq!(plan(&f.core, &second).unwrap(), Action::None);
        assert_eq!(second.moment(1).scheduled, first.moment(1).scheduled);
        let g = Fixture::new();
        g.core.document.lock().unwrap().settings.schedule_hour = 1;
        plan(&g.core, &second).unwrap();
        assert_eq!(g.latest().status, "missed");
    }
}
