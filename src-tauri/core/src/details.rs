use crate::{file_transfer, model::QuarantineItem, quarantine::safe_path, store::*};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileState {
    pub path: String,
    pub state: String,
    pub bytes: Option<u64>,
}
fn state(path: &Path) -> FileState {
    let (status, bytes) = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => ("symlink", None),
        Ok(m) if m.is_file() && safe_path(path).is_ok() => ("present", Some(m.len())),
        Ok(_) => ("unavailable", None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ("absent", None),
        Err(_) => ("unavailable", None),
    };
    FileState {
        path: path.to_string_lossy().into(),
        state: status.into(),
        bytes,
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferDetail {
    pub kind: String,
    pub published: bool,
    pub source: FileState,
    pub destination: FileState,
    pub staged: FileState,
    pub temporary: FileState,
    pub last_error: Option<String>,
    pub attempted_at: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub item: QuarantineItem,
    pub original: FileState,
    pub stored: FileState,
    pub transfers: Vec<TransferDetail>,
    pub integrity: Option<Integrity>,
}
pub fn get(c: &Core, id: &str) -> Result<Detail> {
    let _gate = c.file_gate.try_lock().map_err(|_| "busy")?;
    get_unlocked(c, id)
}
fn get_unlocked(c: &Core, id: &str) -> Result<Detail> {
    let item = c
        .snapshot()
        .quarantine
        .into_iter()
        .find(|q| q.id == id)
        .ok_or("finding_missing")?;
    detail_for_item(c, item)
}
fn detail_for_item(c: &Core, item: QuarantineItem) -> Result<Detail> {
    let id = &item.id;
    let transfers = file_transfer::details(&c.db.lock().unwrap(), id)?
        .into_iter()
        .map(
            |(
                kind,
                published,
                source,
                destination,
                staged,
                temporary,
                last_error,
                attempted_at,
            )| TransferDetail {
                kind,
                published,
                source: state(&source),
                destination: state(&destination),
                staged: state(&staged),
                temporary: state(&temporary),
                last_error,
                attempted_at,
            },
        )
        .collect();
    Ok(Detail {
        original: state(Path::new(&item.original_path)),
        stored: state(&c.root.join("quarantine").join(&item.id)),
        integrity: c
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM runtime_kv WHERE key=?1",
                [format!("integrity:{id}")],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok()),
        item,
        transfers,
    })
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Integrity {
    pub checked_at: String,
    pub state: String,
    pub actual_digest: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub bytes_checked: u64,
    #[serde(default)]
    pub total_bytes: Option<u64>,
}
fn protected_digest_with(
    path: &Path,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(u64, u64),
) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    safe_path(path)?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(err)?;
    let before = file.metadata().map_err(err)?;
    if !before.is_file() {
        return Err("regular_files_only".into());
    }
    progress(0, before.len());
    let mut checked = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        if cancelled() {
            return Err("integrity_cancelled".into());
        }
        let n = file.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        checked = checked.saturating_add(n as u64);
        progress(checked, before.len());
    }
    if cancelled() {
        return Err("integrity_cancelled".into());
    }
    let after = file.metadata().map_err(err)?;
    safe_path(path)?;
    let current = fs::symlink_metadata(path).map_err(err)?;
    if before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
        || current.dev() != after.dev()
        || current.ino() != after.ino()
    {
        return Err("file_changed".into());
    }
    Ok(hex::encode(hash.finalize()))
}
fn check_with(
    path: &Path,
    expected: &str,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(u64, u64),
) -> Integrity {
    let mut result = Integrity {
        checked_at: now(),
        state: "unavailable".into(),
        actual_digest: None,
        error: None,
        bytes_checked: 0,
        total_bytes: None,
    };
    let file = state(path);
    if file.state != "present" {
        result.state = file.state;
        return result;
    }
    match protected_digest_with(path, cancelled, |read, total| {
        result.bytes_checked = read;
        result.total_bytes = Some(total);
        progress(read, total);
    }) {
        Ok(hash) => {
            result.state = if hash == expected {
                "verified"
            } else {
                "mismatch"
            }
            .into();
            result.actual_digest = Some(hash);
        }
        Err(e) if e == "integrity_cancelled" => result.state = "cancelled".into(),
        Err(e) => result.error = Some(e),
    }
    result
}
pub fn stop(c: &Core) {
    use std::sync::atomic::Ordering;
    {
        let mut document = c.document.lock().unwrap();
        if let Some(op) = document
            .operation
            .as_mut()
            .filter(|op| op.kind == "integrity" && op.stage != "finishingIntegrity")
        {
            c.integrity_cancel.store(true, Ordering::Relaxed);
            op.stage = "stoppingIntegrity".into();
        }
    }
    c.publish();
}

pub fn verify(c: &Core, id: &str) -> Result<()> {
    verify_started(c, id, || {})
}

pub(crate) fn verify_started(c: &Core, id: &str, started: impl FnOnce()) -> Result<()> {
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    let _gate = c.file_gate.try_lock().map_err(|_| "busy")?;
    let item = c
        .snapshot()
        .quarantine
        .into_iter()
        .find(|q| q.id == id)
        .ok_or("finding_missing")?;
    {
        let mut d = c.document.lock().unwrap();
        if d.operation.is_some() || d.scan.as_ref().is_some_and(|s| s.status == "running") {
            return Err("busy".into());
        }
        c.integrity_cancel.store(false, Ordering::Relaxed);
        d.operation = Some(crate::model::Operation {
            kind: "integrity".into(),
            target_id: Some(id.into()),
            stage: "checkingIntegrity".into(),
            ..Default::default()
        });
    }
    c.publish();
    started();
    let mut last = Instant::now();
    let mut result = check_with(
        &c.root.join("quarantine").join(&item.id),
        &item.digest,
        || c.integrity_cancel.load(Ordering::Relaxed),
        |read, total| {
            if read == 0 || read == total || last.elapsed() >= Duration::from_millis(150) {
                if let Some(op) = &mut c.document.lock().unwrap().operation {
                    op.downloaded = read;
                    op.total = Some(total);
                }
                c.publish();
                last = Instant::now();
            }
        },
    );
    {
        let mut d = c.document.lock().unwrap();
        if c.integrity_cancel.load(Ordering::Relaxed) {
            result.state = "cancelled".into();
            result.actual_digest = None;
            result.error = None;
        }
        if let Some(op) = &mut d.operation {
            op.stage = "finishingIntegrity".into();
        }
    }
    let saved = (|| {
        c.db.lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO runtime_kv(key,value) VALUES (?1,?2)",
                [
                    format!("integrity:{id}"),
                    serde_json::to_string(&result).map_err(err)?,
                ],
            )
            .map_err(err)?;
        Ok(())
    })();
    c.document.lock().unwrap().operation = None;
    c.log("integrity_checked", format!("{id}: {}", result.state));
    saved
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Export {
    pub schema_version: u32,
    pub exported_at: String,
    pub records: Vec<Detail>,
}
pub fn export(c: &Core) -> Result<Export> {
    let _gate = c.file_gate.try_lock().map_err(|_| "busy")?;
    export_unlocked(c)
}
pub(crate) fn export_unlocked(c: &Core) -> Result<Export> {
    let items = c.snapshot().quarantine;
    Ok(Export {
        schema_version: 1,
        exported_at: now(),
        records: items
            .into_iter()
            .map(|item| detail_for_item(c, item))
            .collect::<Result<_>>()?,
    })
}
pub fn write_export(c: &Core, destination: &Path, bytes: &[u8]) -> Result<()> {
    let _gate = c.file_gate.try_lock().map_err(|_| "busy")?;
    write_export_unlocked(c, destination, bytes)
}
pub(crate) fn write_export_unlocked(c: &Core, destination: &Path, bytes: &[u8]) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let parent = fs::canonicalize(destination.parent().ok_or("invalid_path")?).map_err(err)?;
    safe_path(&parent)?;
    let destination = parent.join(destination.file_name().ok_or("invalid_path")?);
    let snapshot = c.snapshot();
    if destination.starts_with(&c.root)
        || snapshot
            .history
            .iter()
            .chain(snapshot.scan.iter())
            .flat_map(|scan| &scan.findings)
            .any(|finding| fs::canonicalize(&finding.path).is_ok_and(|path| path == destination))
        || snapshot
            .quarantine
            .iter()
            .any(|q| fs::canonicalize(&q.original_path).is_ok_and(|p| p == destination))
    {
        return Err("export_protected_path".into());
    }
    if fs::symlink_metadata(&destination).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
    {
        return Err("export_protected_path".into());
    }
    let temporary = parent.join(format!(".clamav-export-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(err)?;
        file.write_all(bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        fs::rename(&temporary, &destination).map_err(err)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
#[cfg(test)]
fn protected_digest(path: &Path) -> Result<String> {
    protected_digest_with(path, || false, |_, _| {})
}
#[cfg(test)]
fn check(path: &Path, expected: &str) -> Integrity {
    check_with(path, expected, || false, |_, _| {})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_after_partial_read_never_returns_a_completed_digest() {
        use std::cell::Cell;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let file = root.join("large");
        fs::write(&file, vec![7u8; 3 * 65536]).unwrap();
        let expected = crate::scanner::digest(&file).unwrap();
        let stop = Cell::new(false);
        let seen = Cell::new(0);
        let result = check_with(
            &file,
            &expected,
            || stop.get(),
            |read, total| {
                assert_eq!(total, 3 * 65536);
                assert!(read >= seen.get());
                seen.set(read);
                if read >= 65536 {
                    stop.set(true);
                }
            },
        );
        assert_eq!(result.state, "cancelled");
        assert!(result.actual_digest.is_none());
        assert_eq!(result.bytes_checked, 65536);
        assert_eq!(result.total_bytes, Some(3 * 65536));
        assert_eq!(crate::scanner::digest(&file).unwrap(), expected);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn export_includes_completed_and_cancelled_records_and_protects_source_and_database() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let c = Core::new(root.join("app-data")).unwrap();
        let original = root.join("original.txt");
        fs::write(&original, b"keep original").unwrap();
        let item = QuarantineItem {
            id: "test-owner".into(),
            original_path: original.to_string_lossy().into(),
            signature: "fixture".into(),
            digest: "expected".into(),
            quarantined_at: now(),
            mode: 0o600,
            status: "active".into(),
        };
        let mut restored = item.clone();
        restored.id = "restored-owner".into();
        restored.status = "restored".into();
        c.document.lock().unwrap().quarantine = vec![item, restored];
        let cancelled = Integrity {
            checked_at: now(),
            state: "cancelled".into(),
            actual_digest: None,
            error: None,
            bytes_checked: 100,
            total_bytes: Some(500),
        };
        c.db.lock()
            .unwrap()
            .execute(
                "INSERT INTO runtime_kv(key,value) VALUES (?1,?2)",
                [
                    "integrity:test-owner".to_string(),
                    serde_json::to_string(&cancelled).unwrap(),
                ],
            )
            .unwrap();
        let json = serde_json::to_value(export(&c).unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["records"].as_array().unwrap().len(), 2);
        assert_eq!(json["records"][0]["integrity"]["state"], "cancelled");
        assert_eq!(json["records"][0]["integrity"]["bytesChecked"], 100);
        assert!(json["records"][0]["integrity"]["actualDigest"].is_null());
        assert_eq!(json["records"][1]["item"]["status"], "restored");
        let bytes = serde_json::to_vec_pretty(&json).unwrap();
        let destination = root.join("report.json");
        write_export(&c, &destination, &bytes).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        assert_eq!(
            write_export(&c, &original, &bytes).unwrap_err(),
            "export_protected_path"
        );
        assert_eq!(
            write_export(&c, &c.root.join("state.sqlite"), &bytes).unwrap_err(),
            "export_protected_path"
        );
        assert_eq!(fs::read(&original).unwrap(), b"keep original");
        {
            let _gate = c.file_gate.lock().unwrap();
            assert!(export(&c).err().unwrap().contains("busy"));
        }
        drop(c);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn manual_check_distinguishes_match_change_missing_and_link_without_altering_files() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let file = root.join("data");
        fs::write(&file, b"original").unwrap();
        let expected = crate::scanner::digest(&file).unwrap();
        let result = check(&file, &expected);
        assert_eq!(result.state, "verified");
        assert_eq!(result.actual_digest.as_deref(), Some(expected.as_str()));
        fs::write(&file, b"changed").unwrap();
        assert_eq!(check(&file, &expected).state, "mismatch");
        assert_eq!(fs::read(&file).unwrap(), b"changed");
        assert_eq!(check(&root.join("absent"), &expected).state, "absent");
        let link = root.join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(check(&link, &expected).state, "symlink");
        assert!(protected_digest(&link).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn presence_does_not_follow_links_or_claim_hash_verification() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let file = root.join("data");
        fs::write(&file, b"small file").unwrap();
        assert_eq!(state(&file).state, "present");
        assert_eq!(state(&file).bytes, Some(10));
        assert_eq!(state(&root.join("missing")).state, "absent");
        let link = root.join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(state(&link).state, "symlink");
        assert_eq!(state(&link).bytes, None);
        let parent_link = root.join("parent-link");
        std::os::unix::fs::symlink(&root, &parent_link).unwrap();
        assert_eq!(state(&parent_link.join("data")).state, "unavailable");
        fs::remove_dir_all(root).unwrap();
    }
}
