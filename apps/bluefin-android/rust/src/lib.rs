//! Android owns JNI conversion only. Transport, authentication and storage stay in the shared core.
use jni::{
    objects::{JClass, JString},
    sys::jstring,
    JNIEnv,
};
use serde_json::{json, Value};
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Mutex,
};
const MAX_INPUT: usize = 65536;
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
static CALL: Mutex<()> = Mutex::new(());
fn dispatch(input: &str) -> Value {
    let result = (|| -> Result<Value, String> {
        if input.len() > MAX_INPUT {
            return Err("Request too large".into());
        }
        let mut v: Value = serde_json::from_str(input).map_err(|_| "Invalid JSON")?;
        if !v.is_object() {
            return Err("Invalid request object".into());
        }
        if v.get("clientPlatform").is_some_and(|p| p != "android") {
            return Err("Android identity cannot be overridden".into());
        }
        v["clientPlatform"] = json!("android");
        // The current core has Apple identity/profiles. Require an explicit parent-owned
        // capability contract before making ANY server request on Android.
        let support = bluefin_apple_core::call(&json!({"operation":"androidSupport"}));
        let qualified = support.as_ref().is_ok_and(|s| {
            s["schemaVersion"] == 1
                && s["clientIdentity"] == "Bluefin Android"
                && s["media3Profiles"] == true
                && s["scopedStorage"] == true
                && s["signOutCleanup"] == true
        });
        if v["operation"] == "androidBridgeStatus" {
            return Ok(json!({"qualified":qualified,"schemaVersion":1}));
        }
        if !qualified {
            return Err("Shared core Android identity, Media3 profile and scoped storage admission are not qualified".into());
        }
        bluefin_apple_core::call(&v)
    })();
    match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error}),
    }
}
#[no_mangle]
pub extern "system" fn Java_org_bluefin_android_NativeCore_invoke(
    mut env: JNIEnv,
    _: JClass,
    input: JString,
) -> jstring {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _guard = CALL.lock().map_err(|_| "Bridge unavailable")?;
        let raw: String = env
            .get_string(&input)
            .map_err(|_| "Invalid JNI string")?
            .into();
        let output = dispatch(&raw).to_string();
        if output.len() > MAX_OUTPUT {
            return Err("Response too large");
        }
        env.new_string(output)
            .map(|s| s.into_raw())
            .map_err(|_| "JNI allocation failed")
    }));
    match result {
        Ok(Ok(s)) => s,
        _ => env
            .new_string(r#"{"ok":false,"error":"Native bridge failure"}"#)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_large_and_invalid_input() {
        assert_eq!(dispatch(&"x".repeat(MAX_INPUT + 1))["ok"], false);
        assert_eq!(dispatch("{")["ok"], false);
    }
    #[test]
    fn bridge_admission_is_target_specific_and_identity_cannot_be_overridden() {
        assert_eq!(
            dispatch(r#"{"operation":"androidBridgeStatus"}"#)["value"]["qualified"],
            cfg!(target_os = "android")
        );
        assert_eq!(
            dispatch(r#"{"operation":"login","server":"https://example.invalid"}"#)["ok"],
            false
        );
        assert_eq!(
            dispatch(
                r#"{"operation":"info","clientPlatform":"apple","server":"https://example.invalid"}"#
            )["ok"],
            false
        );
    }
}
