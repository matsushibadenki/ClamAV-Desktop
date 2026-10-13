//! Advisory process ownership. Every Core acquires this before touching SQLite/recovery.
//! Never unlink the lock file: contenders must keep locking the same inode.
use crate::store::{err, Result};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub(crate) struct Ownership {
    _file: File,
}
impl Ownership {
    #[cfg(unix)]
    pub fn acquire(root: &Path) -> Result<Self> {
        use std::os::{
            fd::AsRawFd,
            unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let uid = unsafe { libc::geteuid() };
        let root_meta = std::fs::symlink_metadata(root).map_err(err)?;
        if !root_meta.is_dir() || root_meta.uid() != uid {
            return Err("invalid_data_owner".into());
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(root.join("owner.lock"))
            .map_err(|e| format!("ownership_lock_failed: {e}"))?;
        let meta = file.metadata().map_err(err)?;
        if !meta.is_file() || meta.uid() != uid || meta.nlink() != 1 || meta.mode() & 0o077 != 0 {
            return Err("invalid_ownership_lock".into());
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err("data_in_use".into());
            }
            return Err(format!("ownership_lock_failed: {e}"));
        }
        // Refuse replaced paths rather than owning an inode other processes cannot lock.
        let named = std::fs::symlink_metadata(root.join("owner.lock")).map_err(err)?;
        if named.dev() != meta.dev() || named.ino() != meta.ino() || named.file_type().is_symlink()
        {
            return Err("invalid_ownership_lock".into());
        }
        Ok(Self { _file: file })
    }
    #[cfg(not(unix))]
    pub fn acquire(_root: &Path) -> Result<Self> {
        Err("ownership_unsupported".into())
    }
}
