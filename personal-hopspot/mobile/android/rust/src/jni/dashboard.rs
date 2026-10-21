use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::engine::{
    apply_interface_edit, built_in_interfaces_json, configured_interfaces_json,
    toggle_interface_hex,
};

fn into_jstring(env: &JNIEnv, value: &str) -> jstring {
    env.new_string(value)
        .map_or(core::ptr::null_mut(), JString::into_raw)
}

/// The user-configured interfaces as JSON, or `{"error": "..."}` when they cannot be read.
#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeConfiguredInterfaces(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let json = configured_interfaces_json()
        .unwrap_or_else(|error| serde_json::json!({ "error": error }).to_string());
    into_jstring(&env, &json)
}

/// Apply one dashboard edit. Returns null on success and a human-readable reason otherwise.
#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeApplyInterfaceEdit(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let request: String = match env.get_string(&request) {
        Ok(request) => request.into(),
        Err(_) => return into_jstring(&env, "request is not a valid string"),
    };
    match apply_interface_edit(&request) {
        Ok(()) => core::ptr::null_mut(),
        Err(error) => into_jstring(&env, &error),
    }
}

#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeBuiltInInterfaces(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    into_jstring(&env, &built_in_interfaces_json())
}

#[no_mangle]
pub extern "system" fn Java_org_personal_hopspot_NativeBridge_nativeToggleBuiltInInterface(
    mut env: JNIEnv,
    _class: JClass,
    id_hex: JString,
) -> jboolean {
    let id_hex: String = match env.get_string(&id_hex) {
        Ok(id_hex) => id_hex.into(),
        Err(_) => return JNI_FALSE,
    };
    if toggle_interface_hex(&id_hex) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}
