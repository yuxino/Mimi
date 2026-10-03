//! No native handles, process-wide reducer state, credentials or provider calls.
//! Each request carries the bounded state owned by its Kotlin caller.

use jni::{objects::JClass, objects::JString, sys::jstring, JNIEnv};
use std::{panic::catch_unwind, ptr};

fn fail(env: &mut JNIEnv<'_>, class: &str, code: &str) -> jstring {
    // Preserve a JVM exception already raised by JNI (for example allocation
    // failure). Never expose request content or parser diagnostics in errors.
    if !env.exception_check().unwrap_or(true) {
        let _ = env.throw_new(class, code);
    }
    ptr::null_mut()
}

#[no_mangle]
pub extern "system" fn Java_app_yuxino_mimi_android_provider_SharedSubtitleCore_exchangeRaw(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    request: JString<'_>,
) -> jstring {
    let request: String = match env.get_string(&request) {
        Ok(value) => value.into(),
        Err(_) => {
            return fail(
                &mut env,
                "java/lang/IllegalArgumentException",
                "shared_core_invalid_request",
            );
        }
    };
    let response = match catch_unwind(|| mimi_core::bridge::exchange(&request)) {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => {
            return fail(
                &mut env,
                "java/lang/IllegalArgumentException",
                "shared_core_invalid_request",
            );
        }
        Err(_) => {
            return fail(
                &mut env,
                "java/lang/IllegalStateException",
                "shared_core_failed",
            );
        }
    };
    match env.new_string(response) {
        Ok(value) => value.into_raw(),
        Err(_) => fail(
            &mut env,
            "java/lang/IllegalStateException",
            "shared_core_response_failed",
        ),
    }
}
