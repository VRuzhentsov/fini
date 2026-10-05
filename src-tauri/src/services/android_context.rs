//! Shared Android JNI plumbing for Fini's own Kotlin classes
//! (`com.fini.app.SyncForegroundService`).
//!
//! 1. **The Android context.** `ndk_context::initialize_android_context`
//!    may run only once per process, and `tauri-plugin-ble-gatt` needs it
//!    too, so Fini goes through the plugin's idempotent
//!    `android_context::ensure()` instead of initializing it itself.
//! 2. **Loading app-defined classes from a natively-attached thread.**
//!    `FindClass` (used implicitly by class-name strings in most `jni-rs`
//!    calls) only searches the bootstrap classloader when called from a
//!    thread the JVM did not create itself — exactly the case for every
//!    thread here, attached via `attach_current_thread_as_daemon`. The
//!    bootstrap classloader only finds core Android framework classes,
//!    never app-defined ones, so classes are resolved through the app's own
//!    classloader instead.

use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};

/// Makes `ndk_context::android_context()` usable. Only after the
/// Activity is up; see `tauri_plugin_ble_gatt::android_context`.
pub fn ensure_bridged() -> Result<(), String> {
    #[cfg(feature = "ui-plane")]
    {
        tauri_plugin_ble_gatt::android_context::ensure()
    }
    #[cfg(not(feature = "ui-plane"))]
    {
        Err("the Android context needs the ui-plane build".to_string())
    }
}

/// Resolves an app-defined class through the Context's own classloader —
/// see the module doc for why `env.find_class(binary_name)` can't be used
/// here instead.
///
/// `binary_name` must be a **dotted** Java binary name (`"com.fini.app.SyncForegroundService"`),
/// not the JNI-internal slash form (`"com/fini/app/SyncForegroundService"`) —
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
/// Context): Unit` — e.g. `SyncForegroundService.start`.
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
