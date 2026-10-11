//! Durable cross-volume transfer. The source stays in a private staging directory
//! until the owning quarantine row commits. Recovery rolls back uncommitted work.
use crate::{scanner, store::*};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Journal {
    id: String,
    owner: String,
    kind: String,
    source: PathBuf,
    destination: PathBuf,
    staging_dir: PathBuf,
    temporary_dir: PathBuf,
    digest: String,
    published: bool,
}
impl Journal {
    fn staged(&self) -> PathBuf {
        self.staging_dir.join("source")
    }
    fn temporary(&self) -> PathBuf {
        self.temporary_dir.join("copy")
    }
    fn save(&self, db: &Connection) -> Result<()> {
        save_row(db, "file_transfers", &self.id, self)
    }
}
pub fn different_volume(source: &Path, destination: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::metadata(source).map_err(err)?.dev()
        != fs::metadata(destination.parent().ok_or("invalid_path")?)
            .map_err(err)?
            .dev())
}
fn sync_dir(path: &Path) -> Result<()> {
    fs::File::open(path).map_err(err)?.sync_all().map_err(err)
}
fn matches(path: &Path, hash: &str) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        && scanner::digest(path).is_ok_and(|h| h == hash)
}
fn create_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    fs::create_dir(path).map_err(err)?;
    let result = (|| {
        private_dir(path)?;
        let meta = fs::metadata(path).map_err(err)?;
        if meta.permissions().mode() & 0o777 != 0o700 || meta.uid() != unsafe { libc::geteuid() } {
            return Err("transfer_private_permissions_unsupported".into());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir(path);
    }
    result
}
fn empty_dir(path: &Path) {
    let _ = fs::remove_dir(path);
}
fn delete_journal(db: &Connection, j: &Journal) -> Result<()> {
    db.execute("DELETE FROM file_transfers WHERE id=?1", [&j.id])
        .map_err(err)?;
    Ok(())
}
pub fn prepare(
    db: &Connection,
    owner: &str,
    kind: &str,
    source: &Path,
    destination: &Path,
    hash: &str,
    verify: impl FnOnce(&Path) -> Result<()>,
) -> Result<Journal> {
    crate::quarantine::safe_path(source)?;
    crate::quarantine::safe_path(destination.parent().ok_or("invalid_path")?)?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err("restore_destination_exists".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let j = Journal {
        staging_dir: source
            .parent()
            .ok_or("invalid_path")?
            .join(format!(".clamav-transfer-{id}-source")),
        temporary_dir: destination
            .parent()
            .ok_or("invalid_path")?
            .join(format!(".clamav-transfer-{id}-copy")),
        id,
        owner: owner.into(),
        kind: kind.into(),
        source: source.into(),
        destination: destination.into(),
        digest: hash.into(),
        published: false,
    };
    // Establish ownership before journalling; no existing directory is reused.
    create_private(&j.staging_dir)?;
    if let Err(e) = create_private(&j.temporary_dir) {
        empty_dir(&j.staging_dir);
        return Err(e);
    }
    if let Err(e) = j.save(db) {
        empty_dir(&j.staging_dir);
        empty_dir(&j.temporary_dir);
        return Err(e);
    }
    let mut j = j;
    let result: Result<()> = (|| {
        crate::quarantine::move_exclusive(&j.source, &j.staged())?;
        sync_dir(&j.staging_dir)?;
        sync_dir(j.source.parent().unwrap())?;
        if !matches(&j.staged(), hash) {
            return Err("file_changed".into());
        }
        use std::os::unix::fs::OpenOptionsExt;
        let mut input = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(j.staged())
            .map_err(err)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(j.temporary())
            .map_err(err)?;
        std::io::copy(&mut input, &mut output).map_err(err)?;
        output.sync_all().map_err(err)?;
        if !matches(&j.temporary(), hash) || !matches(&j.staged(), hash) {
            return Err("file_changed".into());
        }
        verify(&j.temporary())?;
        crate::quarantine::move_exclusive(&j.temporary(), &j.destination)?;
        j.published = true;
        j.save(db)?;
        sync_dir(j.destination.parent().unwrap())?;
        Ok(())
    })();
    if let Err(e) = result {
        let recovery = rollback(db, &j);
        return Err(format!("{e}; transfer_rollback={recovery:?}"));
    }
    Ok(j)
}
/// Called only after the owning row has durably changed to active/restored.
pub fn finish(db: &Connection, j: &Journal) -> Result<()> {
    if j.staging_dir.exists() {
        crate::quarantine::safe_path(&j.staging_dir)?;
    }
    crate::quarantine::safe_path(&j.destination)?;
    if !matches(&j.destination, &j.digest) {
        return Err("transfer_destination_changed".into());
    }
    if fs::symlink_metadata(j.staged()).is_ok() {
        if !matches(&j.staged(), &j.digest) {
            return Err("transfer_source_changed".into());
        }
        fs::remove_file(j.staged()).map_err(err)?;
        sync_dir(&j.staging_dir)?;
    }
    if j.temporary().exists() {
        crate::quarantine::safe_path(&j.temporary())?;
        if !matches(&j.temporary(), &j.digest) {
            return Err("transfer_temporary_changed".into());
        }
        fs::remove_file(j.temporary()).map_err(err)?;
    }
    empty_dir(&j.staging_dir);
    empty_dir(&j.temporary_dir);
    sync_dir(j.source.parent().ok_or("invalid_path")?)?;
    delete_journal(db, j)
}
fn rollback(db: &Connection, j: &Journal) -> Result<()> {
    crate::quarantine::safe_path(j.source.parent().ok_or("invalid_path")?)?;
    if j.staging_dir.exists() {
        crate::quarantine::safe_path(&j.staging_dir)?;
    }
    if fs::symlink_metadata(j.staged()).is_ok() {
        // Restore the actual staged source even when its bytes changed. Never overwrite.
        crate::quarantine::move_exclusive(&j.staged(), &j.source)?;
        sync_dir(j.source.parent().ok_or("invalid_path")?)?;
    }
    if !j.source.is_file() {
        return Err("transfer_source_unavailable".into());
    }
    if j.published {
        if !matches(&j.source, &j.digest) {
            return Err("transfer_source_changed".into());
        }
        if fs::symlink_metadata(&j.destination).is_ok() {
            if !matches(&j.destination, &j.digest) {
                return Err("transfer_destination_changed".into());
            }
            crate::quarantine::safe_path(&j.destination)?;
            fs::remove_file(&j.destination).map_err(err)?;
            sync_dir(j.destination.parent().ok_or("invalid_path")?)?;
        }
    } else if fs::symlink_metadata(&j.destination).is_ok() {
        return Err("transfer_needs_review".into());
    }
    if fs::symlink_metadata(j.temporary()).is_ok() {
        crate::quarantine::safe_path(&j.temporary())?;
        fs::remove_file(j.temporary()).map_err(err)?;
    }
    empty_dir(&j.staging_dir);
    empty_dir(&j.temporary_dir);
    delete_journal(db, j)
}
pub fn recover(db: &Connection) -> Result<Vec<String>> {
    let journals: Vec<Journal> = {
        let mut st = db.prepare("SELECT json FROM file_transfers").map_err(err)?;
        let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
        rows.map(|r| serde_json::from_str(&r.map_err(err)?).map_err(err))
            .collect::<Result<_>>()?
    };
    let mut pending = vec![];
    for j in journals {
        let state: Option<String> = db
            .query_row("SELECT json FROM quarantine WHERE id=?1", [&j.owner], |r| {
                r.get(0)
            })
            .ok();
        let mut item =
            state.and_then(|s| serde_json::from_str::<crate::model::QuarantineItem>(&s).ok());
        let committed = item.as_ref().is_some_and(|q| {
            (j.kind == "quarantine" && q.status == "active")
                || (j.kind == "restore" && q.status == "restored")
        });
        let result = if committed {
            finish(db, &j)
        } else {
            rollback(db, &j)
        };
        if result.is_err() {
            pending.push(j.owner);
        } else if !committed {
            if let Some(q) = &mut item {
                q.status = if j.kind == "restore" {
                    "active"
                } else {
                    "move_failed"
                }
                .into();
                save_row(db, "quarantine", &q.id, q)?;
            }
        }
    }
    Ok(pending)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Connection, PathBuf, PathBuf, String) {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let source = root.join("source");
        let dest = root.join("dest");
        fs::write(&source, b"harmless transfer fixture").unwrap();
        let hash = scanner::digest(&source).unwrap();
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE file_transfers(id TEXT PRIMARY KEY,json TEXT NOT NULL); CREATE TABLE quarantine(id TEXT PRIMARY KEY,json TEXT NOT NULL);").unwrap();
        (root, db, source, dest, hash)
    }
    #[test]
    #[ignore = "requires a separate mounted test volume via CLAMAV_TEST_VOLUME"]
    fn actual_cross_volume_round_trip() {
        let (root, db, _, _, _) = fixture();
        let volume = fs::canonicalize(
            std::env::var("CLAMAV_TEST_VOLUME").expect("CLAMAV_TEST_VOLUME required"),
        )
        .unwrap();
        let external = volume.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&external).unwrap();
        let source = external.join("検証: harmless.txt");
        let destination = root.join("quarantined");
        fs::write(&source, b"harmless external volume transfer test").unwrap();
        let hash = scanner::digest(&source).unwrap();
        assert!(different_volume(&source, &destination).unwrap());
        let j = prepare(
            &db,
            "test",
            "quarantine",
            &source,
            &destination,
            &hash,
            |_| Ok(()),
        )
        .unwrap();
        assert!(matches(&destination, &hash));
        assert!(!source.exists());
        finish(&db, &j).unwrap();
        let j = prepare(&db, "test", "restore", &destination, &source, &hash, |_| {
            Ok(())
        })
        .unwrap();
        finish(&db, &j).unwrap();
        assert!(matches(&source, &hash));
        assert!(!destination.exists());
        fs::remove_dir_all(external).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_finishes_a_committed_transfer() {
        let (root, db, src, dest, hash) = fixture();
        let j = prepare(&db, "owner", "quarantine", &src, &dest, &hash, |_| Ok(())).unwrap();
        let item = crate::model::QuarantineItem {
            id: "owner".into(),
            original_path: src.to_string_lossy().into(),
            signature: "test".into(),
            digest: hash.clone(),
            quarantined_at: now(),
            mode: 0o600,
            status: "active".into(),
        };
        save_row(&db, "quarantine", "owner", &item).unwrap();
        assert!(recover(&db).unwrap().is_empty());
        assert!(matches(&dest, &hash));
        assert!(!j.staged().exists());
        assert!(recover(&db).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn keeps_source_until_commit_and_finishes_verified_copy() {
        let (root, db, src, dest, hash) = fixture();
        let j = prepare(&db, "owner", "quarantine", &src, &dest, &hash, |_| Ok(())).unwrap();
        assert!(!src.exists());
        assert!(matches(&dest, &hash));
        assert!(matches(&j.staged(), &hash));
        finish(&db, &j).unwrap();
        assert!(matches(&dest, &hash));
        assert!(!j.staged().exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_verification_restores_source() {
        let (root, db, src, dest, hash) = fixture();
        assert!(
            prepare(&db, "owner", "quarantine", &src, &dest, &hash, |_| Err(
                "test rejection".into()
            ))
            .is_err()
        );
        assert!(matches(&src, &hash));
        assert!(!dest.exists());
        assert_eq!(
            db.query_row("SELECT count(*) FROM file_transfers", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_rolls_back_uncommitted_published_copy() {
        let (root, db, src, dest, hash) = fixture();
        prepare(&db, "owner", "quarantine", &src, &dest, &hash, |_| Ok(())).unwrap();
        assert!(recover(&db).unwrap().is_empty());
        assert!(matches(&src, &hash));
        assert!(!dest.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_completes_an_interrupted_rollback() {
        let (root, db, src, dest, hash) = fixture();
        let j = prepare(&db, "owner", "quarantine", &src, &dest, &hash, |_| Ok(())).unwrap();
        crate::quarantine::move_exclusive(&j.staged(), &src).unwrap();
        fs::remove_file(&dest).unwrap();
        assert!(recover(&db).unwrap().is_empty());
        assert!(matches(&src, &hash));
        assert!(recover(&db).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recovery_preserves_source_when_original_path_is_reoccupied() {
        let (root, db, src, dest, hash) = fixture();
        let j = prepare(&db, "owner", "quarantine", &src, &dest, &hash, |_| Ok(())).unwrap();
        fs::write(&src, b"new user file").unwrap();
        assert_eq!(recover(&db).unwrap(), vec!["owner"]);
        assert_eq!(fs::read(&src).unwrap(), b"new user file");
        assert!(matches(&j.staged(), &hash));
        assert!(matches(&dest, &hash));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn never_replaces_existing_destination() {
        let (root, db, src, dest, hash) = fixture();
        fs::write(&dest, b"existing").unwrap();
        assert!(prepare(&db, "owner", "restore", &src, &dest, &hash, |_| Ok(())).is_err());
        assert!(matches(&src, &hash));
        assert_eq!(fs::read(&dest).unwrap(), b"existing");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn source_change_is_preserved_on_rejection() {
        let (root, db, src, dest, hash) = fixture();
        fs::write(&src, b"changed file").unwrap();
        assert!(prepare(&db, "owner", "quarantine", &src, &dest, &hash, |_| Ok(())).is_err());
        assert_eq!(fs::read(&src).unwrap(), b"changed file");
        assert!(!dest.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
