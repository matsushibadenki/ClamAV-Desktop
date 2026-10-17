//! Advisory process ownership. Every Core acquires this before touching SQLite/recovery.
//! Never unlink the lock file: contenders must keep locking the same inode.
use crate::store::{err, Result};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub(crate) struct Ownership {
    _file: File,
    owner_pid: libc::pid_t,
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
        let ownership = Self {
            _file: file,
            owner_pid: unsafe { libc::getpid() },
        };
        // Refuse replaced paths rather than owning an inode other processes cannot lock.
        let named = std::fs::symlink_metadata(root.join("owner.lock")).map_err(err)?;
        if named.dev() != meta.dev() || named.ino() != meta.ino() || named.file_type().is_symlink()
        {
            return Err("invalid_ownership_lock".into());
        }
        Ok(ownership)
    }
    #[cfg(not(unix))]
    pub fn acquire(_root: &Path) -> Result<Self> {
        Err("ownership_unsupported".into())
    }
}

impl Drop for Ownership {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // CLOEXEC does not close fork-inherited descriptors until exec. Closing
            // only this descriptor can therefore leave the lock briefly alive.
            // Core drops DB/document first; its acquiring process explicitly unlocks.
            if unsafe { libc::getpid() } == self.owner_pid {
                loop {
                    if unsafe { libc::flock(self._file.as_raw_fd(), libc::LOCK_UN) } == 0 {
                        break;
                    }
                    if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn inherited_guard_cannot_unlock_the_acquiring_process() {
        let root = std::path::PathBuf::from("/private/tmp")
            .join(format!("clamav-inherited-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let owner = Ownership::acquire(&root).unwrap();
        let inherited = Ownership {
            _file: owner._file.try_clone().unwrap(),
            owner_pid: unsafe { libc::getpid() } + 1,
        };
        drop(inherited);
        assert_eq!(Ownership::acquire(&root).err().unwrap(), "data_in_use");
        drop(owner);
        let owner = Ownership::acquire(&root).unwrap();
        drop(owner);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn forked_child_descriptor_cannot_extend_released_ownership() {
        let root = std::path::PathBuf::from("/private/tmp")
            .join(format!("clamav-fork-owner-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let owner = Ownership::acquire(&root).unwrap();
        let mut pipe = [0; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            // Between fork and exec only async-signal-safe syscalls are permitted.
            unsafe {
                libc::close(pipe[1]);
                let mut byte = 0u8;
                libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1);
                libc::_exit(0);
            }
        }
        assert!(pid > 0);
        unsafe {
            libc::close(pipe[0]);
        }
        // The child deliberately keeps the inherited descriptor open, as can happen
        // briefly before exec in another thread's process spawn.
        drop(owner);
        let reacquired = Ownership::acquire(&root);
        unsafe {
            let byte = 1u8;
            libc::write(pipe[1], (&byte as *const u8).cast(), 1);
            libc::close(pipe[1]);
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }
        assert!(
            reacquired.is_ok(),
            "fork-inherited descriptor retained ownership: {:?}",
            reacquired.err()
        );
        drop(reacquired);
        // A real active owner must still exclude a second open of the same inode.
        let active = Ownership::acquire(&root).unwrap();
        assert_eq!(Ownership::acquire(&root).err().unwrap(), "data_in_use");
        assert_ne!(
            unsafe { libc::fcntl(active._file.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        drop(active);
        std::fs::remove_dir_all(root).unwrap();
    }
}
