use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

use jni::objects::JClass;
use jni::sys::jint;
use jni::JNIEnv;

use crate::engine::adopt_local_client;

const ADOPTED: jint = 0;
const INVALID_DESCRIPTOR: jint = 3;

/// Take ownership of a detached socketpair descriptor from `PrnsHubService` and attach it as a
/// shared-instance local client. The descriptor is closed on every path, including failure.
#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeAdoptLocalClient(
    _env: JNIEnv,
    _class: JClass,
    fd: jint,
    uid: jint,
) -> jint {
    if fd < 0 {
        return INVALID_DESCRIPTOR;
    }
    // SAFETY: the Kotlin caller obtained `fd` from `ParcelFileDescriptor.detachFd()`, which
    // relinquishes ownership, and never touches it again; this is the descriptor's only owner.
    let descriptor = unsafe { OwnedFd::from_raw_fd(fd) };
    match adopt_local_client(UnixStream::from(descriptor), uid as u32) {
        Ok(()) => ADOPTED,
        Err(error) => error.code(),
    }
}
