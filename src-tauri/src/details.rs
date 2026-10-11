use crate::{file_transfer, model::QuarantineItem, quarantine::safe_path, store::*};
use serde::Serialize;
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
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub item: QuarantineItem,
    pub original: FileState,
    pub stored: FileState,
    pub transfers: Vec<TransferDetail>,
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
            |(kind, published, source, destination, staged, temporary)| TransferDetail {
                kind,
                published,
                source: state(&source),
                destination: state(&destination),
                staged: state(&staged),
                temporary: state(&temporary),
            },
        )
        .collect();
    Ok(Detail {
        original: state(Path::new(&item.original_path)),
        stored: state(&c.root.join("quarantine").join(&item.id)),
        item,
        transfers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
