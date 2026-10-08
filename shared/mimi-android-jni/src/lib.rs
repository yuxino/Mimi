//! JNI adapters for the shared subtitle reducer and live provider runtime.
//! Android owns capture and UI; Rust owns provider and subtitle policy.

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

mod runtime;

#[no_mangle]
pub extern "system" fn Java_app_yuxino_mimi_android_provider_NativeRuntimeConfiguration_exchangeRaw(
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
                "native_runtime_invalid_request",
            )
        }
    };
    let response = match catch_unwind(|| runtime::exchange(&request)) {
        Ok(Ok(value)) => value,
        Ok(Err(code)) => return fail(&mut env, "java/lang/IllegalArgumentException", code),
        Err(_) => {
            return fail(
                &mut env,
                "java/lang/IllegalStateException",
                "native_runtime_failed",
            )
        }
    };
    match env.new_string(response) {
        Ok(value) => value.into_raw(),
        Err(_) => fail(
            &mut env,
            "java/lang/IllegalStateException",
            "native_runtime_failed",
        ),
    }
}

#[no_mangle]
pub extern "system" fn Java_app_yuxino_mimi_android_provider_NativeRuntimeConfiguration_pcmRaw(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jni::sys::jlong,
    pcm: jni::objects::JByteArray<'_>,
) -> jni::sys::jboolean {
    if !matches!(env.get_array_length(&pcm), Ok(0..=192_000)) {
        let _ = fail(
            &mut env,
            "java/lang/IllegalArgumentException",
            "native_runtime_invalid_pcm",
        );
        return 0;
    }
    let pcm = match env.convert_byte_array(&pcm) {
        Ok(value) => value,
        Err(_) => return 0,
    };
    match catch_unwind(|| runtime::pcm(handle as u64, pcm)) {
        Ok(Ok(value)) => value as u8,
        Ok(Err(code)) => {
            let _ = fail(&mut env, "java/lang/IllegalArgumentException", code);
            0
        }
        Err(_) => {
            let _ = fail(
                &mut env,
                "java/lang/IllegalStateException",
                "native_runtime_failed",
            );
            0
        }
    }
}
