//! Shared Android JNI plumbing.
//!
//! Two things kept here rather than duplicated at each call site:
//!
//! 1. **Bridging `tao`'s Android context into `ndk-context`.**
//!    `ndk_context::initialize_android_context` panics if called more than
//!    once, so every caller that needs it (`transport::ble`'s lazy backend,
//!    the OS-pairing check below) must go through one shared, idempotent
//!    entry point rather than each running their own copy.
//! 2. **Loading app-defined classes from a natively-attached thread.**
//!    `FindClass` (used implicitly by class-name strings in most `jni-rs`
//!    calls) only searches the bootstrap classloader when called from a
//!    thread the JVM did not create itself — exactly the case for every
//!    thread here, attached via `attach_current_thread_as_daemon`. The
//!    bootstrap classloader only finds core Android framework classes,
//!    never app-defined ones; `ble_gatt::backend::android` hit this
//!    directly (`ClassNotFoundException: Didn't find class
//!    "dev.blegatt.BleGattBridge"`) and works around it by resolving
//!    classes through the app's own classloader instead. Same fix here.

use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};
use std::sync::OnceLock;

/// Bridges `tao`'s Android context into the `ndk-context` crate's global,
/// exactly once for the process's lifetime — safe to call from multiple
/// independent sites.
///
/// Must only be called from a point guaranteed to run after the
/// Activity/WebView is up (see `transport::ble`'s module doc for why
/// `.setup()` itself is too early to read `ndk_context::android_context()`);
/// every current caller only runs from the first real `space_sync_tick`
/// invocation onward.
pub fn ensure_bridged() -> Result<(), String> {
    static BRIDGED: OnceLock<Result<(), String>> = OnceLock::new();
    BRIDGED
        .get_or_init(|| {
            use tao::platform::android::prelude::main_android_context;

            let ctx = main_android_context()
                .ok_or_else(|| "tao's Android context is not available yet".to_string())?;
            unsafe {
                ndk_context::initialize_android_context(ctx.java_vm, ctx.context_jobject);
            }
            Ok(())
        })
        .clone()
}

/// Resolves an app-defined class through the Context's own classloader —
/// see the module doc for why `env.find_class(binary_name)` can't be used
/// here instead.
///
/// `binary_name` must be a **dotted** Java binary name (`"com.fini.app.BluetoothPairing"`),
/// not the JNI-internal slash form (`"com/fini/app/BluetoothPairing"`) —
/// `ClassLoader.loadClass(String)` is a normal Java reflection call, not
/// `FindClass`, and only accepts the dotted form. Passing the slash form
/// throws `ClassNotFoundException` for every call through this function,
/// silently, since every caller below fails closed on error.
fn load_app_class<'a>(
    env: &mut JNIEnv<'a>, context: &JObject, binary_name: &str,
) -> Result<jni::objects::JClass<'a>, String> {
    let context_class = env.get_object_class(context).map_err(|err| err.to_string())?;
    let class_loader = env
        .call_method(&context_class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
        .and_then(|v| v.l())
        .map_err(|err| format!("getClassLoader failed: {err}"))?;
    let name = env.new_string(binary_name).map_err(|err| err.to_string())?;
    let class_obj = env
        .call_method(&class_loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[
            JValue::Object(&name),
        ])
        .and_then(|v| v.l())
        .map_err(|err| format!("loadClass({binary_name}) failed: {err}"))?;
    Ok(jni::objects::JClass::from(class_obj))
}

/// Attaches the current thread and resolves `ctx.context()` as a `JObject`,
/// shared setup for every `call_static_*` helper below.
fn resolve_context<'local>() -> Result<(JavaVM, JObject<'local>), String> {
    ensure_bridged()?;
    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
        .map_err(|err| format!("JavaVM::from_raw failed: {err}"))?;
    let context_obj = unsafe { JObject::from_raw(ctx.context().cast()) };
    Ok((vm, context_obj))
}

/// Calls an app-defined Kotlin object's `@JvmStatic fun name(context:
/// Context, address: String): Boolean` — e.g. `BluetoothPairing.isBonded`.
/// `class_binary_name` is dotted (`"com.fini.app.BluetoothPairing"`) — see
/// `load_app_class`'s doc comment.
///
/// Fails closed (`false`) on any error along the way: an unreachable check
/// (bridge not ready, class not found, permission denied inside the Kotlin
/// method) must never be mistaken for a positive pairing result.
pub fn call_static_context_string_to_bool(class_binary_name: &str, method: &str, arg: &str) -> bool {
    let (vm, context_obj) = match resolve_context() {
        Ok(attached) => attached,
        Err(err) => {
            eprintln!("[android-context] bridge unavailable, failing closed: {err}");
            return false;
        }
    };
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(env) => env,
        Err(err) => {
            eprintln!("[android-context] JNI attach failed, failing closed: {err}");
            return false;
        }
    };

    let class = match load_app_class(&mut env, &context_obj, class_binary_name) {
        Ok(class) => class,
        Err(err) => {
            eprintln!("[android-context] loading {class_binary_name} failed, failing closed: {err}");
            return false;
        }
    };
    let jarg = match env.new_string(arg) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[android-context] new_string failed, failing closed: {err}");
            return false;
        }
    };
    let result = env.call_static_method(
        class,
        method,
        "(Landroid/content/Context;Ljava/lang/String;)Z",
        &[JValue::Object(&context_obj), JValue::Object(&jarg)],
    );
    match result.and_then(|v| v.z()) {
        Ok(value) => value,
        Err(err) => {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            eprintln!(
                "[android-context] {class_binary_name}.{method} failed, failing closed: {err}"
            );
            false
        }
    }
}

/// Calls an app-defined Kotlin object's `@JvmStatic fun name(context:
/// Context, address: String): Boolean?` — the tri-state form
/// `call_static_context_string_to_bool` collapses away. Kotlin's nullable
/// `Boolean?` marshals as a boxed `java.lang.Boolean`, so `null` is a
/// distinct wire value from `false`, not the same thing squashed together.
///
/// Returns `None` for the Kotlin method's own `null` result *and* for
/// every failure along the way (bridge unavailable, class not found,
/// exception thrown inside the method) -- none of those are evidence of
/// `false`, only that this check couldn't be completed. Use this instead
/// of the plain-bool variant wherever the caller needs to tell "confirmed
/// false" apart from "inconclusive" (e.g.
/// `device_connection::commands::persist_bluetooth_address_and_maybe_enable`);
/// callers that are fine failing closed either way should keep using
/// `call_static_context_string_to_bool`.
pub fn call_static_context_string_to_optional_bool(
    class_binary_name: &str,
    method: &str,
    arg: &str,
) -> Option<bool> {
    let (vm, context_obj) = match resolve_context() {
        Ok(attached) => attached,
        Err(err) => {
            eprintln!("[android-context] bridge unavailable, treating as inconclusive: {err}");
            return None;
        }
    };
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(env) => env,
        Err(err) => {
            eprintln!("[android-context] JNI attach failed, treating as inconclusive: {err}");
            return None;
        }
    };

    let class = match load_app_class(&mut env, &context_obj, class_binary_name) {
        Ok(class) => class,
        Err(err) => {
            eprintln!(
                "[android-context] loading {class_binary_name} failed, treating as inconclusive: {err}"
            );
            return None;
        }
    };
    let jarg = match env.new_string(arg) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[android-context] new_string failed, treating as inconclusive: {err}");
            return None;
        }
    };
    let result = env.call_static_method(
        class,
        method,
        "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/Boolean;",
        &[JValue::Object(&context_obj), JValue::Object(&jarg)],
    );
    let boxed = match result.and_then(|v| v.l()) {
        Ok(obj) => obj,
        Err(err) => {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            eprintln!(
                "[android-context] {class_binary_name}.{method} failed, treating as inconclusive: {err}"
            );
            return None;
        }
    };
    if boxed.is_null() {
        return None;
    }
    match env.call_method(&boxed, "booleanValue", "()Z", &[]).and_then(|v| v.z()) {
        Ok(value) => Some(value),
        Err(err) => {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            eprintln!(
                "[android-context] unboxing {class_binary_name}.{method}'s result failed, treating as inconclusive: {err}"
            );
            None
        }
    }
}

/// Calls an app-defined Kotlin object's `@JvmStatic fun name(context:
/// Context): Boolean` — e.g. `BluetoothPairing.hasPermissions`. Fails closed
/// (`false`), same reasoning as the string-argument variant above.
pub fn call_static_context_to_bool(class_binary_name: &str, method: &str) -> bool {
    let (vm, context_obj) = match resolve_context() {
        Ok(attached) => attached,
        Err(err) => {
            eprintln!("[android-context] bridge unavailable, failing closed: {err}");
            return false;
        }
    };
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(env) => env,
        Err(err) => {
            eprintln!("[android-context] JNI attach failed, failing closed: {err}");
            return false;
        }
    };

    let class = match load_app_class(&mut env, &context_obj, class_binary_name) {
        Ok(class) => class,
        Err(err) => {
            eprintln!("[android-context] loading {class_binary_name} failed, failing closed: {err}");
            return false;
        }
    };
    let result =
        env.call_static_method(class, method, "(Landroid/content/Context;)Z", &[JValue::Object(
            &context_obj,
        )]);
    match result.and_then(|v| v.z()) {
        Ok(value) => value,
        Err(err) => {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            eprintln!(
                "[android-context] {class_binary_name}.{method} failed, failing closed: {err}"
            );
            false
        }
    }
}

/// Calls an app-defined Kotlin object's `@JvmStatic fun name(context:
/// Context): Unit` — e.g. `BluetoothPairing.requestPermissionsIfNeeded`.
/// Fire-and-forget: there is no synchronous result to return, only the
/// eventual system permission dialog outcome, which the user resolves in
/// their own time. Errors are logged, not propagated — a failed request
/// attempt leaves the app exactly where it already was (ungranted).
pub fn call_static_context_void(class_binary_name: &str, method: &str) {
    let (vm, context_obj) = match resolve_context() {
        Ok(attached) => attached,
        Err(err) => {
            eprintln!("[android-context] bridge unavailable, not calling {class_binary_name}.{method}: {err}");
            return;
        }
    };
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(env) => env,
        Err(err) => {
            eprintln!("[android-context] JNI attach failed, not calling {class_binary_name}.{method}: {err}");
            return;
        }
    };

    let class = match load_app_class(&mut env, &context_obj, class_binary_name) {
        Ok(class) => class,
        Err(err) => {
            eprintln!("[android-context] loading {class_binary_name} failed: {err}");
            return;
        }
    };
    if let Err(err) =
        env.call_static_method(class, method, "(Landroid/content/Context;)V", &[JValue::Object(
            &context_obj,
        )])
    {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        eprintln!("[android-context] {class_binary_name}.{method} failed: {err}");
    }
}

/// Calls an app-defined Kotlin object's `@JvmStatic fun name(context:
/// Context, arg: String): Unit` — e.g. `BluetoothPairing.createBond`.
/// Fire-and-forget, same reasoning as `call_static_context_void`: bonding
/// completes asynchronously via a system broadcast the caller here never
/// observes, so there is nothing meaningful to return.
pub fn call_static_context_string_void(class_binary_name: &str, method: &str, arg: &str) {
    let (vm, context_obj) = match resolve_context() {
        Ok(attached) => attached,
        Err(err) => {
            eprintln!("[android-context] bridge unavailable, not calling {class_binary_name}.{method}: {err}");
            return;
        }
    };
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(env) => env,
        Err(err) => {
            eprintln!("[android-context] JNI attach failed, not calling {class_binary_name}.{method}: {err}");
            return;
        }
    };

    let class = match load_app_class(&mut env, &context_obj, class_binary_name) {
        Ok(class) => class,
        Err(err) => {
            eprintln!("[android-context] loading {class_binary_name} failed: {err}");
            return;
        }
    };
    let jarg = match env.new_string(arg) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[android-context] new_string failed: {err}");
            return;
        }
    };
    if let Err(err) = env.call_static_method(
        class,
        method,
        "(Landroid/content/Context;Ljava/lang/String;)V",
        &[JValue::Object(&context_obj), JValue::Object(&jarg)],
    ) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        eprintln!("[android-context] {class_binary_name}.{method} failed: {err}");
    }
}
