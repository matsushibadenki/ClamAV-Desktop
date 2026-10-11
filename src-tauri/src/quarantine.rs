use crate::{model::*, scanner, store::*};
use std::{fs, path::Path};
use tauri::AppHandle;
#[cfg(target_os = "macos")]
pub(crate) fn move_exclusive(from: &Path, to: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(from.as_os_str().as_bytes()).map_err(err)?;
    let to = CString::new(to.as_os_str().as_bytes()).map_err(err)?;
    let code = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if code != 0 {
        return Err(format!(
            "file_move_failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn move_exclusive(_from: &Path, _to: &Path) -> Result<()> {
    Err("macos_only".into())
}
pub(crate) fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err("invalid_path".into());
    }
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component);
        if fs::symlink_metadata(&current)
            .map_err(err)?
            .file_type()
            .is_symlink()
        {
            return Err("symlink_not_allowed".into());
        }
    }
    Ok(())
}
pub fn quarantine(app: &AppHandle, id: &str) -> Result<()> {
    let c = core(app);
    let _gate = c.file_gate.lock().unwrap();
    let finding = c
        .snapshot()
        .history
        .iter()
        .flat_map(|s| s.findings.iter())
        .find(|f| f.id == id && !f.quarantined)
        .cloned()
        .ok_or("finding_missing")?;
    let source = Path::new(&finding.path);
    safe_path(source)?;
    if source.starts_with(&c.root) {
        return Err("app_data_target".into());
    }
    let hash = finding.digest.ok_or("scan_hash_missing")?;
    if scanner::digest(source)? != hash {
        return Err("file_changed".into());
    }
    let meta = fs::metadata(source).map_err(err)?;
    if !meta.is_file() {
        return Err("regular_files_only".into());
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    };
    #[cfg(not(unix))]
    let mode = 0;
    let dest = c.root.join("quarantine").join(id);
    let mut item = QuarantineItem {
        id: id.into(),
        original_path: finding.path.clone(),
        signature: finding.signature,
        digest: hash,
        quarantined_at: now(),
        mode,
        status: "moving".into(),
    };
    save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
    let transfer = if crate::file_transfer::different_volume(source, &dest)? {
        let snapshot = c.snapshot();
        let prepared = crate::file_transfer::prepare(
            &c.db.lock().unwrap(),
            id,
            "quarantine",
            source,
            &dest,
            &item.digest,
            |p| confirm_detection(&snapshot, p, &item.signature),
        );
        match prepared {
            Ok(j) => Some(j),
            Err(e) => {
                item.status = "move_failed".into();
                save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
                refresh_items(app)?;
                return Err(e);
            }
        }
    } else {
        if let Err(e) = move_exclusive(source, &dest) {
            item.status = "move_failed".into();
            save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
            return Err(e);
        }
        None
    };
    if scanner::digest(&dest)? != item.digest {
        let rollback = move_exclusive(&dest, source);
        item.status = "changed".into();
        save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
        return Err(format!("file_changed: rollback={rollback:?}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o600)).map_err(err)?;
    }
    // Re-scan the moved, private file to bind the detection to the exact bytes
    // being quarantined. The source could have changed while ClamAV was scanning.
    if let Err(reason) = if transfer.is_some() {
        Ok(())
    } else {
        confirm_detection(&c.snapshot(), &dest, &item.signature)
    } {
        match move_exclusive(&dest, source) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(source, fs::Permissions::from_mode(item.mode))
                        .map_err(err)?;
                }
                item.status = "move_failed".into();
            }
            Err(_) => item.status = "review".into(),
        }
        save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
        {
            let mut d = c.document.lock().unwrap();
            d.quarantine.retain(|q| q.id != id);
            d.quarantine.insert(0, item);
        }
        c.publish(app);
        return Err(reason);
    }
    item.status = "active".into();
    save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
    if let Some(j) = transfer {
        let finished = crate::file_transfer::finish(&c.db.lock().unwrap(), &j);
        if let Err(e) = finished {
            refresh_items(app)?;
            return Err(e);
        }
    }
    {
        let mut d = c.document.lock().unwrap();
        d.quarantine.retain(|q| q.id != id);
        d.quarantine.insert(0, item);
        for s in &mut d.history {
            for f in &mut s.findings {
                if f.id == id {
                    f.quarantined = true;
                }
            }
            c.save_scan(s)?;
        }
        if let Some(s) = d.scan.as_mut() {
            for f in &mut s.findings {
                if f.id == id {
                    f.quarantined = true
                }
            }
        }
    }
    c.log(app, "file_quarantined", source.to_string_lossy());
    Ok(())
}
pub fn restore(app: &AppHandle, id: &str) -> Result<()> {
    let c = core(app);
    let _gate = c.file_gate.lock().unwrap();
    let mut item = c
        .snapshot()
        .quarantine
        .iter()
        .find(|q| q.id == id && (q.status == "active" || q.status == "review"))
        .cloned()
        .ok_or("quarantine_missing")?;
    let dest = Path::new(&item.original_path);
    safe_path(dest.parent().ok_or("invalid_path")?)?;
    if fs::symlink_metadata(dest).is_ok() {
        return Err("restore_destination_exists".into());
    }
    let source = c.root.join("quarantine").join(id);
    safe_path(&source)?;
    if scanner::digest(&source)? != item.digest {
        return Err("quarantine_hash_mismatch".into());
    }
    let original_status = item.status.clone();
    item.status = "restoring".into();
    save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
    let transfer = if crate::file_transfer::different_volume(&source, dest)? {
        let prepared = crate::file_transfer::prepare(
            &c.db.lock().unwrap(),
            id,
            "restore",
            &source,
            dest,
            &item.digest,
            |_| Ok(()),
        );
        match prepared {
            Ok(j) => Some(j),
            Err(e) => {
                item.status = original_status;
                save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
                refresh_items(app)?;
                return Err(e);
            }
        }
    } else {
        if let Err(e) = move_exclusive(&source, dest) {
            item.status = original_status;
            save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
            return Err(e);
        }
        None
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dest, fs::Permissions::from_mode(item.mode)).map_err(err)?;
    }
    item.status = "restored".into();
    save_row(&c.db.lock().unwrap(), "quarantine", id, &item)?;
    if let Some(j) = transfer {
        let finished = crate::file_transfer::finish(&c.db.lock().unwrap(), &j);
        if let Err(e) = finished {
            refresh_items(app)?;
            return Err(e);
        }
    }
    {
        let mut d = c.document.lock().unwrap();
        if let Some(q) = d.quarantine.iter_mut().find(|q| q.id == id) {
            *q = item.clone();
        }
    }
    refresh_items(app)?;
    c.log(app, "file_restored", dest.to_string_lossy());
    Ok(())
}
fn confirm_detection(snapshot: &Snapshot, path: &Path, signature: &str) -> Result<()> {
    use std::process::{Command, Stdio};
    let binary = crate::engine::discover().ok_or("engine_missing")?;
    let db = snapshot
        .engine
        .database_path
        .as_deref()
        .ok_or("definitions_missing")?;
    let mut child = Command::new(binary)
        .args(["--stdout", "--infected", "--follow-file-symlinks=0"])
        .arg(format!("--database={db}"))
        .arg(format!("--max-filesize={}M", snapshot.settings.max_file_mb))
        .arg("--")
        .arg(path)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(err)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if child.try_wait().map_err(err)?.is_some() {
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("quarantine_rescan_timeout".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let output = child.wait_with_output().map_err(err)?;
    let matched = String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| matches!(scanner::parse(line),scanner::Parsed::Found(_,sig) if sig==signature));
    if output.status.code() != Some(1) || !matched {
        return Err("file_changed".into());
    }
    Ok(())
}

pub fn reconcile(
    root: &Path,
    db: &rusqlite::Connection,
    items: &mut [QuarantineItem],
) -> Result<()> {
    for item in items {
        let source = root.join("quarantine").join(&item.id);
        if matches!(
            item.status.as_str(),
            "moving" | "restoring" | "active" | "review"
        ) {
            if source.is_file() {
                if safe_path(&source).is_err() {
                    item.status = "integrity_error".into();
                    save_row(db, "quarantine", &item.id, item)?;
                    continue;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).map_err(err)?;
                }
                if scanner::digest(&source)? == item.digest {
                    // Interrupted moves need explicit review instead of an assumed verdict.
                    if item.status == "moving" {
                        item.status = "review".into();
                    }
                    if item.status == "restoring" {
                        item.status = "active".into();
                    }
                } else {
                    item.status = "integrity_error".into();
                }
            } else if item.status == "restoring"
                && Path::new(&item.original_path).is_file()
                && scanner::digest(Path::new(&item.original_path)).is_ok_and(|h| h == item.digest)
            {
                item.status = "restored".into();
            } else if item.status == "moving" {
                item.status = "move_failed".into();
            } else {
                item.status = "missing".into();
            }
            save_row(db, "quarantine", &item.id, item)?;
        }
    }
    Ok(())
}

pub fn refresh_items(app: &AppHandle) -> Result<()> {
    let c = core(app);
    let db = c.db.lock().unwrap();
    let pending = crate::file_transfer::recover(&db)?;
    let mut items: Vec<QuarantineItem> = crate::store::read_rows(&db, "quarantine")?;
    reconcile(&c.root, &db, &mut items)?;
    for item in &mut items {
        if pending.contains(&item.id) {
            item.status = "transfer_pending".into();
        }
    }
    drop(db);
    items.sort_by(|a, b| b.quarantined_at.cmp(&a.quarantined_at));
    {
        let mut d = c.document.lock().unwrap();
        for scan in &mut d.history {
            for finding in &mut scan.findings {
                finding.quarantined = items.iter().any(|q| {
                    q.id == finding.id
                        && matches!(q.status.as_str(), "active" | "review" | "transfer_pending")
                });
            }
            c.save_scan(scan)?;
        }
        if let Some(scan) = &mut d.scan {
            for finding in &mut scan.findings {
                finding.quarantined = items.iter().any(|q| {
                    q.id == finding.id
                        && matches!(q.status.as_str(), "active" | "review" | "transfer_pending")
                });
            }
        }
        d.quarantine = items;
    }
    c.publish(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn never_overwrites_destination() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        fs::write(&a, b"source").unwrap();
        fs::write(&b, b"existing").unwrap();
        assert!(move_exclusive(&a, &b).is_err());
        assert_eq!(fs::read(&a).unwrap(), b"source");
        assert_eq!(fs::read(&b).unwrap(), b"existing");
        fs::remove_dir_all(root).unwrap();
    }
}
