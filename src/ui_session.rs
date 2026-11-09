//! Process-wide desktop exclusivity. Kept outside the ELM327 library.
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

/// Hold this guard until the GUI event loop and its workers have stopped.
/// OS locks are released even on a crash. Never unlink the lock file: replacing
/// its inode would let another process acquire a different lock at the same path.
pub(crate) struct UiSession {
    file: File,
}

impl Drop for UiSession {
    fn drop(&mut self) {
        // Explicitly unlock before closing. A concurrent process spawn may
        // briefly inherit the descriptor on Unix until its exec closes it.
        let _ = self.file.unlock();
    }
}

impl UiSession {
    pub(crate) fn acquire(name: &str) -> io::Result<Self> {
        #[cfg(target_os = "windows")]
        let home = std::env::var_os("LOCALAPPDATA");
        #[cfg(not(target_os = "windows"))]
        let home = std::env::var_os("HOME");
        let directory = home
            .map(std::path::PathBuf::from)
            .ok_or_else(|| io::Error::other("Cannot locate the Kittx application data directory"))?
            .join(".kittx");
        std::fs::create_dir_all(&directory)?;
        Self::at(&directory.join(format!("{name}.lock")))
    }

    fn at(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { file }),
            Err(TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Another Kittx UI is already open. Close it before opening another UI.",
            )),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

#[cfg(test)]
#[path = "ui_session_tests.rs"]
mod tests;
