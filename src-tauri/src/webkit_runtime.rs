const WEBKIT_DISABLE_DMABUF_RENDERER: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
const WEBKIT_DISABLE_SANDBOX: &str = "WEBKIT_DISABLE_SANDBOX";

pub fn apply_startup_guards() {
    set_default_env(WEBKIT_DISABLE_DMABUF_RENDERER, "1");
    set_default_env(WEBKIT_DISABLE_SANDBOX, "1");
}

fn set_default_env(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn with_env_guard<F>(test: F)
    where
        F: FnOnce(),
    {
        let _guard = env_lock();
        let dmabuf = std::env::var_os(WEBKIT_DISABLE_DMABUF_RENDERER);
        let sandbox = std::env::var_os(WEBKIT_DISABLE_SANDBOX);

        std::env::remove_var(WEBKIT_DISABLE_DMABUF_RENDERER);
        std::env::remove_var(WEBKIT_DISABLE_SANDBOX);

        test();

        restore_env(WEBKIT_DISABLE_DMABUF_RENDERER, dmabuf);
        restore_env(WEBKIT_DISABLE_SANDBOX, sandbox);
    }

    fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn applies_default_webkit_guards() {
        with_env_guard(|| {
            apply_startup_guards();

            assert_eq!(std::env::var(WEBKIT_DISABLE_DMABUF_RENDERER).unwrap(), "1");
            assert_eq!(std::env::var(WEBKIT_DISABLE_SANDBOX).unwrap(), "1");
        });
    }

    #[test]
    fn preserves_explicit_webkit_guard_overrides() {
        with_env_guard(|| {
            std::env::set_var(WEBKIT_DISABLE_DMABUF_RENDERER, "0");
            std::env::set_var(WEBKIT_DISABLE_SANDBOX, "0");

            apply_startup_guards();

            assert_eq!(std::env::var(WEBKIT_DISABLE_DMABUF_RENDERER).unwrap(), "0");
            assert_eq!(std::env::var(WEBKIT_DISABLE_SANDBOX).unwrap(), "0");
        });
    }
}
