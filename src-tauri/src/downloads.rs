//! Only aged, recognizable installer attempts inside the private cache are removable.
use crate::{quarantine::safe_path, store::*};
use serde::Serialize;
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};
const RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheReport {
    pub bytes: u64,
    pub reclaimable_bytes: u64,
    pub attempts: usize,
    pub removable: usize,
    pub skipped: usize,
}
fn installer_file(name: &str) -> bool {
    let package = name.strip_suffix(".sig").unwrap_or(name);
    let Some(version) = package
        .strip_prefix("clamav-")
        .and_then(|v| v.strip_suffix(".macos.universal.pkg"))
    else {
        return false;
    };
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
fn inspect(dir: &Path, protected: Option<&Path>, now: SystemTime) -> Result<Option<(u64, bool)>> {
    let meta = fs::symlink_metadata(dir).map_err(err)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Ok(None);
    }
    safe_path(dir)?;
    let mut newest = meta.modified().map_err(err)?;
    let mut bytes = 0u64;
    for entry in fs::read_dir(dir).map_err(err)? {
        let path = entry.map_err(err)?.path();
        let meta = fs::symlink_metadata(&path).map_err(err)?;
        let name = path.file_name().unwrap().to_string_lossy();
        if !meta.is_file() || meta.file_type().is_symlink() || !installer_file(&name) {
            return Ok(None);
        }
        newest = newest.max(meta.modified().map_err(err)?);
        bytes = bytes.saturating_add(meta.len());
    }
    let old = now.duration_since(newest).is_ok_and(|age| age >= RETENTION);
    let active = protected.is_some_and(|p| p.parent() == Some(dir));
    Ok(Some((bytes, old && !active)))
}
fn manage(
    root: &Path,
    protected: Option<&Path>,
    now: SystemTime,
    remove: bool,
) -> Result<CacheReport> {
    safe_path(root)?;
    let mut report = CacheReport {
        bytes: 0,
        reclaimable_bytes: 0,
        attempts: 0,
        removable: 0,
        skipped: 0,
    };
    for entry in fs::read_dir(root).map_err(err)? {
        let path = entry.map_err(err)?.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| uuid::Uuid::parse_str(n).ok())
            .is_none()
        {
            report.skipped += 1;
            continue;
        }
        match inspect(&path, protected, now)? {
            Some((bytes, eligible)) => {
                report.attempts += 1;
                report.bytes = report.bytes.saturating_add(bytes);
                if eligible {
                    report.removable += 1;
                    report.reclaimable_bytes = report.reclaimable_bytes.saturating_add(bytes);
                    if remove {
                        fs::remove_dir_all(&path).map_err(err)?;
                    }
                }
            }
            None => report.skipped += 1,
        }
    }
    Ok(report)
}
pub fn report(c: &Core) -> Result<CacheReport> {
    let prepared = c.prepared.lock().unwrap();
    manage(
        &c.root.join("downloads"),
        prepared.as_ref().map(|(p, _)| p.as_path()),
        SystemTime::now(),
        false,
    )
}
pub fn cleanup(app: &tauri::AppHandle) -> Result<CacheReport> {
    let c = core(app);
    let _gate = c.file_gate.lock().unwrap();
    {
        let mut d = c.document.lock().unwrap();
        if d.operation.is_some() {
            return Err("busy".into());
        }
        d.operation = Some(crate::model::Operation {
            kind: "cache_cleanup".into(),
            stage: "cleaning".into(),
            ..Default::default()
        });
    }
    c.publish(app);
    let result = {
        let prepared = c.prepared.lock().unwrap();
        manage(
            &c.root.join("downloads"),
            prepared.as_ref().map(|(p, _)| p.as_path()),
            SystemTime::now(),
            true,
        )
    };
    c.document.lock().unwrap().operation = None;
    c.log(
        app,
        if result.is_ok() {
            "cache_cleaned"
        } else {
            "error"
        },
        result
            .as_ref()
            .map(|r| r.reclaimable_bytes.to_string())
            .unwrap_or_else(|e| e.clone()),
    );
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_only_old_known_attempts_and_preserves_prepared_unknown_and_symlinks() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let old = root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&old).unwrap();
        fs::write(old.join("clamav-1.4.0.macos.universal.pkg"), b"12345").unwrap();
        let active = root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&active).unwrap();
        let pkg = active.join("clamav-1.5.4.macos.universal.pkg");
        fs::write(&pkg, b"ready").unwrap();
        let unknown = root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("user.txt"), b"keep").unwrap();
        let link = root.join(uuid::Uuid::new_v4().to_string());
        std::os::unix::fs::symlink(&unknown, &link).unwrap();
        let now = SystemTime::now();
        assert_eq!(manage(&root, Some(&pkg), now, true).unwrap().removable, 0);
        let future = now + RETENTION + Duration::from_secs(2);
        let preview = manage(&root, Some(&pkg), future, false).unwrap();
        assert_eq!(preview.reclaimable_bytes, 5);
        assert_eq!(preview.skipped, 2);
        assert!(old.exists());
        assert_eq!(
            manage(&root, Some(&pkg), future, true).unwrap().removable,
            1
        );
        assert!(!old.exists());
        assert!(pkg.exists());
        assert!(unknown.join("user.txt").exists());
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        fs::remove_dir_all(root).unwrap();
    }
}
