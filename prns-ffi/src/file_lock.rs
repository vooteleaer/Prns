//! Advisory whole-file locks through `flock(2)` on Haiku, where Rust's std
//! does not yet implement `File::try_lock` / `File::unlock` (it uses `flock`
//! on most other Unix systems). Same semantics: one lock per open file
//! description, released on unlock or when it closes.

use std::fs::{File, TryLockError};
use std::io;
use std::os::fd::AsRawFd;

/// As `File::try_lock`: an exclusive lock, or `WouldBlock` if another open
/// file description holds one.
pub fn try_lock(file: &File) -> Result<(), TryLockError> {
    match flock(file, libc::LOCK_EX | libc::LOCK_NB) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(TryLockError::WouldBlock),
        Err(error) => Err(TryLockError::Error(error)),
    }
}

/// As `File::unlock`.
pub fn unlock(file: &File) -> io::Result<()> {
    flock(file, libc::LOCK_UN)
}

fn flock(file: &File, operation: libc::c_int) -> io::Result<()> {
    // SAFETY: `file` owns the descriptor and keeps it open for the duration of
    // the call; flock takes no pointers.
    if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
