const WEBKIT_DISABLE_DMABUF_RENDERER: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
const WEBKIT_DISABLE_SANDBOX: &str = "WEBKIT_DISABLE_SANDBOX";
const WEBKIT_DISABLE_COMPOSITING_MODE: &str = "WEBKIT_DISABLE_COMPOSITING_MODE";

const GUARD_KEYS: [&str; 3] = [
    WEBKIT_DISABLE_DMABUF_RENDERER,
    WEBKIT_DISABLE_SANDBOX,
    WEBKIT_DISABLE_COMPOSITING_MODE,
];

pub fn apply_startup_guards() {
    set_default_env(WEBKIT_DISABLE_DMABUF_RENDERER, "1");
    set_default_env(WEBKIT_DISABLE_SANDBOX, "1");
    set_default_env(WEBKIT_DISABLE_COMPOSITING_MODE, "1");
}

fn set_default_env(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

/// Logs the resolved value of each WebKit startup guard to stderr so a
/// packaged AppImage session can confirm which flags actually reached
/// `WebKitWebProcess`, without recording any host- or session-identifying
/// data.
pub fn log_active_guards() {
    for key in GUARD_KEYS {
        match std::env::var(key) {
            Ok(value) => eprintln!("[webkit-runtime] {key}={value}"),
            Err(_) => eprintln!("[webkit-runtime] {key}=<unset>"),
        }
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
        let saved: Vec<_> = GUARD_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();

        for key in GUARD_KEYS {
            std::env::remove_var(key);
        }

        test();

        for (key, value) in saved {
            restore_env(key, value);
        }
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
            assert_eq!(std::env::var(WEBKIT_DISABLE_COMPOSITING_MODE).unwrap(), "1");
        });
    }

    #[test]
    fn preserves_explicit_webkit_guard_overrides() {
        with_env_guard(|| {
            std::env::set_var(WEBKIT_DISABLE_DMABUF_RENDERER, "0");
            std::env::set_var(WEBKIT_DISABLE_SANDBOX, "0");
            std::env::set_var(WEBKIT_DISABLE_COMPOSITING_MODE, "0");

            apply_startup_guards();

            assert_eq!(std::env::var(WEBKIT_DISABLE_DMABUF_RENDERER).unwrap(), "0");
            assert_eq!(std::env::var(WEBKIT_DISABLE_SANDBOX).unwrap(), "0");
            assert_eq!(std::env::var(WEBKIT_DISABLE_COMPOSITING_MODE).unwrap(), "0");
        });
    }
}
