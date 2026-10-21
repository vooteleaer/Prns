use std::time::Duration;

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jbyteArray, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::rnode_ble::{self, Outbound};

/// Block up to `timeout_millis` for the next RNode connect request, encoded as
/// `"<session>\t<target>"` (see `rnode_ble::ConnectRequest`), or null when none arrived.
#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeRNodeBleNextRequest(
    env: JNIEnv,
    _class: JClass,
    timeout_millis: jlong,
) -> jstring {
    let timeout = Duration::from_millis(timeout_millis.max(0) as u64);
    rnode_ble::next_request(timeout)
        .and_then(|request| {
            env.new_string(format!("{}\t{}", request.session, request.target))
                .ok()
        })
        .map_or(core::ptr::null_mut(), JString::into_raw)
}

/// Report the GATT link for `session` ready (null `error`) or failed.
#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeRNodeBleOpened(
    mut env: JNIEnv,
    _class: JClass,
    session: jint,
    error: JString,
) {
    let result = if error.is_null() {
        Ok(())
    } else {
        Err(env
            .get_string(&error)
            .map(String::from)
            .unwrap_or_else(|_| "Bluetooth connection failed".to_string()))
    };
    rnode_ble::opened(session as u32, result);
}

#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeRNodeBleInbound(
    env: JNIEnv,
    _class: JClass,
    session: jint,
    bytes: JByteArray,
) -> jboolean {
    let delivered = env
        .convert_byte_array(&bytes)
        .is_ok_and(|bytes| rnode_ble::inbound(session as u32, bytes));
    if delivered {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// Block up to `timeout_millis` for bytes to write to the RNode. Returns the bytes, an empty
/// array when nothing is pending, or null once the session is closed.
#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeRNodeBleTakeOutbound(
    env: JNIEnv,
    _class: JClass,
    session: jint,
    timeout_millis: jlong,
) -> jbyteArray {
    let timeout = Duration::from_millis(timeout_millis.max(0) as u64);
    let bytes = match rnode_ble::take_outbound(session as u32, timeout) {
        Outbound::Bytes(bytes) => bytes,
        Outbound::Idle => Vec::new(),
        Outbound::Closed => return core::ptr::null_mut(),
    };
    env.byte_array_from_slice(&bytes)
        .map_or(core::ptr::null_mut(), JByteArray::into_raw)
}

#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeRNodeBleDisconnected(
    _env: JNIEnv,
    _class: JClass,
    session: jint,
) {
    rnode_ble::disconnected(session as u32);
}
