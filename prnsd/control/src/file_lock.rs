//! Advisory file locks, as `File::try_lock` and `File::unlock`. Rust's std
//! implements those with `flock(2)` on most Unix systems but not yet on
//! Haiku, which has `flock` too: there prns-ffi calls it (the FFI stays out
//! of this crate).

use std::fs::{File, TryLockError};
use std::io;

#[cfg(not(target_os = "haiku"))]
pub(crate) fn try_lock(file: &File) -> Result<(), TryLockError> {
    file.try_lock()
}

#[cfg(not(target_os = "haiku"))]
pub(crate) fn unlock(file: &File) -> io::Result<()> {
    file.unlock()
}

#[cfg(target_os = "haiku")]
pub(crate) fn try_lock(file: &File) -> Result<(), TryLockError> {
    prns_ffi::file_lock::try_lock(file)
}

#[cfg(target_os = "haiku")]
pub(crate) fn unlock(file: &File) -> io::Result<()> {
    prns_ffi::file_lock::unlock(file)
}
