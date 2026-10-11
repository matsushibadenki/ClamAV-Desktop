use crate::{file_transfer, model::QuarantineItem, quarantine::safe_path, store::*};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
#[derive(Serialize)]
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
#[derive(Serialize)]
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
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub item: QuarantineItem,
    pub original: FileState,
    pub stored: FileState,
    pub transfers: Vec<TransferDetail>,
    pub integrity: Option<Integrity>,
}
pub fn get(c: &Core, id: &str) -> Result<Detail> {
    let _gate = c.file_gate.lock().unwrap();
    let item = c
        .snapshot()
        .quarantine
        .into_iter()
        .find(|q| q.id == id)
        .ok_or("finding_missing")?;
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Integrity {
    pub checked_at: String,
    pub state: String,
    pub actual_digest: Option<String>,
    pub error: Option<String>,
}
fn protected_digest(path: &Path) -> Result<String> {
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
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
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
fn check(path: &Path, expected: &str) -> Integrity {
    let mut result = Integrity {
        checked_at: now(),
        state: "unavailable".into(),
        actual_digest: None,
        error: None,
    };
    let file = state(path);
    if file.state != "present" {
        result.state = file.state;
        return result;
    }
    match protected_digest(path) {
        Ok(hash) => {
            result.state = if hash == expected {
                "verified"
            } else {
                "mismatch"
            }
            .into();
            result.actual_digest = Some(hash);
        }
        Err(e) => result.error = Some(e),
    }
    result
}
pub fn verify(app: &tauri::AppHandle, id: &str) -> Result<()> {
    let c = core(app);
    let _gate = c.file_gate.lock().unwrap();
    let item = c
        .snapshot()
        .quarantine
        .into_iter()
        .find(|q| q.id == id)
        .ok_or("finding_missing")?;
    let result = check(&c.root.join("quarantine").join(&item.id), &item.digest);
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
    c.log(app, "integrity_checked", format!("{id}: {}", result.state));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
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
