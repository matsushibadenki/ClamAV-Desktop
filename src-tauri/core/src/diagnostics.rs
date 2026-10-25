use crate::{model::*, store::*};
use serde::{Deserialize, Serialize};
const KINDS: [&str; 4] = [
    "engine_inspection",
    "engine_install",
    "engine_launch",
    "definitions_update",
];
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub at: String,
    pub kind: String,
    pub status: String,
    pub code: String,
    pub detail: Option<String>,
    #[serde(default)]
    pub retry_not_before: Option<String>,
}
pub fn classify(reason: &str) -> &'static str {
    let lower = reason.to_lowercase();
    let status = |code: &str| {
        [
            format!("http {code}"),
            format!("http error {code}"),
            format!("error code {code}"),
            format!("({code} "),
            format!("({code})"),
            format!("response {code}"),
            format!("status {code}"),
        ]
        .iter()
        .any(|pattern| lower.contains(pattern))
    };
    if lower.contains("definitions_update_cancelled") {
        "cancelled"
    } else if status("429")
        || lower.contains("cool-down")
        || lower.contains("cooldown")
        || lower.contains("retry after")
    {
        "rate_limit"
    } else if status("403")
        || status("503")
        || status("1020")
        || lower.contains("outdated")
        || lower.contains("unsupported")
    {
        "engine_update_required"
    } else if lower.contains("download_cache_limit_exceeded") {
        "cache_limit"
    } else if lower.contains("insufficient_space")
        || lower.contains("no space left")
        || lower.contains("os error 28")
    {
        "disk_full"
    } else if lower.contains("engine_missing") || lower.contains("freshclam_missing") {
        "engine_missing"
    } else if lower.contains("permission denied")
        || lower.contains("read-only")
        || lower.contains("os error 13")
    {
        "access_denied"
    } else if lower.contains("pgp")
        || lower.contains("signature")
        || lower.contains("gatekeeper")
        || lower.contains("package_changed")
        || lower.contains("untrusted")
    {
        "verification_failed"
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else if lower.contains("dns")
        || lower.contains("resolve")
        || lower.contains("connect")
        || lower.contains("http")
        || lower.contains("request")
    {
        "network"
    } else if lower.contains("definitions_missing") {
        "definitions_invalid"
    } else {
        "unknown"
    }
}
fn retry_time(reason: &str) -> String {
    use chrono::TimeZone;
    for line in reason.lines() {
        if let Some((_, value)) = line.split_once("until after:") {
            if let Some(value) = value.trim().get(..19) {
                if let Ok(time) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                {
                    if let Some(local) = chrono::Local.from_local_datetime(&time).single() {
                        if local > chrono::Local::now() {
                            return local.with_timezone(&chrono::Utc).to_rfc3339();
                        }
                    }
                }
            }
        }
    }
    // Conservative app backoff when FreshClam supplies no usable retry time.
    (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339()
}
pub fn retry_blocked(c: &Core, kind: &str) -> bool {
    c.snapshot().diagnostics.iter().any(|a| {
        a.kind == kind
            && a.status == "failed"
            && a.code == "rate_limit"
            && a.retry_not_before
                .as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_some_and(|time| time > chrono::Utc::now())
    })
}
pub fn load(db: &rusqlite::Connection) -> Result<Vec<Attempt>> {
    let mut records = vec![];
    for kind in KINDS {
        let value = db
            .query_row(
                "SELECT value FROM runtime_kv WHERE key=?1",
                [format!("diagnostic:{kind}")],
                |r| r.get::<_, String>(0),
            )
            .ok();
        if let Some(record) = value.and_then(|v| serde_json::from_str::<Attempt>(&v).ok()) {
            records.push(record);
        }
    }
    records.sort_by(|a, b| b.at.cmp(&a.at));
    Ok(records)
}
pub fn save(c: &Core, kind: &str, reason: Option<&str>) -> Result<()> {
    if !KINDS.contains(&kind) {
        return Err("invalid_diagnostic_kind".into());
    }
    let attempt = Attempt {
        at: now(),
        kind: kind.into(),
        status: if reason.is_some() {
            "failed"
        } else {
            "succeeded"
        }
        .into(),
        code: reason.map(classify).unwrap_or("ok").into(),
        detail: reason.map(|r| r.chars().take(8192).collect()),
        retry_not_before: reason
            .filter(|r| classify(r) == "rate_limit")
            .map(retry_time),
    };
    let persisted =
        c.db.lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO runtime_kv(key,value) VALUES (?1,?2)",
                [
                    format!("diagnostic:{kind}"),
                    serde_json::to_string(&attempt).map_err(err)?,
                ],
            )
            .map_err(err);
    let mut d = c.document.lock().unwrap();
    d.diagnostics.retain(|a| a.kind != kind);
    d.diagnostics.push(attempt);
    d.diagnostics.sort_by(|a, b| b.at.cmp(&a.at));
    persisted.map(|_| ())
}
pub fn record<T>(c: &Core, kind: &str, result: &Result<T>) {
    let _ = save(&c, kind, result.as_ref().err().map(String::as_str));
    c.publish();
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema_version: u32,
    pub exported_at: String,
    pub app_version: String,
    pub os: String,
    pub architecture: String,
    pub state: Snapshot,
    pub quarantine_details: Option<crate::details::Export>,
    pub download_cache: Option<crate::downloads::CacheReport>,
    pub scheduled_runs: Option<Vec<crate::scheduler::Run>>,
    pub collection_errors: Vec<String>,
}
pub fn report(c: &Core) -> Result<Report> {
    report_with(c, crate::details::export(c))
}
pub(crate) fn report_unlocked(c: &Core) -> Result<Report> {
    report_with(c, crate::details::export_unlocked(c))
}
fn report_with(c: &Core, quarantine: Result<crate::details::Export>) -> Result<Report> {
    let mut collection_errors = vec![];
    let quarantine_details = match quarantine {
        Ok(value) => Some(value),
        Err(reason) if reason == "busy" => return Err(reason),
        Err(reason) => {
            collection_errors.push(format!("quarantine: {reason}"));
            None
        }
    };
    let download_cache = match crate::downloads::report(c) {
        Ok(value) => Some(value),
        Err(reason) => {
            collection_errors.push(format!("download_cache: {reason}"));
            None
        }
    };
    let scheduled_runs = match crate::scheduler::records(&c.db.lock().unwrap()) {
        Ok(records) => Some(records),
        Err(reason) => {
            collection_errors.push(format!("scheduled_runs: {reason}"));
            None
        }
    };
    Ok(Report {
        scheduled_runs,
        schema_version: 1,
        exported_at: now(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        os: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        state: c.snapshot(),
        quarantine_details,
        download_cache,
        collection_errors,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_rate_limit_verification_space_and_network_without_immediate_retry() {
        assert_eq!(
            classify("freshclam_failed: HTTP 429 cool-down until tomorrow"),
            "rate_limit"
        );
        assert_eq!(
            classify("gatekeeper_rejected_package"),
            "verification_failed"
        );
        assert_eq!(
            classify("No space left on device (os error 28)"),
            "disk_full"
        );
        assert_eq!(classify("Could not resolve database.clamav.net"), "network");
        assert_eq!(classify("process_timeout"), "timeout");
        assert_eq!(
            classify("pgp_signature_invalid: abc429def"),
            "verification_failed"
        );
    }
    #[test]
    fn persists_latest_attempt_and_report_does_not_read_file_contents() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let c = Core::new(root.clone()).unwrap();
        save(&c, "definitions_update", Some("HTTP 429")).unwrap();
        assert_eq!(load(&c.db.lock().unwrap()).unwrap()[0].code, "rate_limit");
        assert!(retry_blocked(&c, "definitions_update"));
        save(&c, "definitions_update", None).unwrap();
        assert_eq!(c.snapshot().diagnostics.len(), 1);
        assert_eq!(c.snapshot().diagnostics[0].status, "succeeded");
        let json = serde_json::to_value(report(&c).unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert!(json["quarantineDetails"]["records"]
            .as_array()
            .unwrap()
            .is_empty());
        std::fs::remove_dir(c.root.join("downloads")).unwrap();
        let partial = serde_json::to_value(report(&c).unwrap()).unwrap();
        assert!(partial["downloadCache"].is_null());
        assert_eq!(partial["collectionErrors"].as_array().unwrap().len(), 1);
        drop(c);
        let reopened = Core::new(root.clone()).unwrap();
        assert_eq!(reopened.snapshot().diagnostics[0].status, "succeeded");
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
