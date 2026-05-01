use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::convert::TryFrom;
use std::thread;

use crate::models::UpsertSettingInput;
use crate::schema::settings;
use crate::services::db::DbState;
#[cfg(target_os = "linux")]
use gtk::prelude::GtkSettingsExt;
use tauri::{AppHandle, Emitter, Manager, Theme};

const THEME_KEY: &str = "theme";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

fn upsert_setting(conn: &mut SqliteConnection, key: &str, value: &str) -> Result<(), String> {
    diesel::insert_into(settings::table)
        .values(UpsertSettingInput {
            key: key.to_string(),
            value: value.to_string(),
        })
        .on_conflict(settings::key)
        .do_update()
        .set(settings::value.eq(value))
        .execute(conn)
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn load_setting(conn: &mut SqliteConnection, key: &str) -> Result<Option<String>, String> {
    settings::table
        .filter(settings::key.eq(key))
        .select(settings::value)
        .first::<String>(conn)
        .optional()
        .map_err(|e| e.to_string())
}

pub fn theme_mode(conn: &mut SqliteConnection) -> Result<ThemeMode, String> {
    let value = match load_setting(conn, THEME_KEY)? {
        Some(value) => value,
        None => {
            upsert_setting(conn, THEME_KEY, ThemeMode::System.as_str())?;
            ThemeMode::System.as_str().to_string()
        }
    };

    Ok(ThemeMode::parse(&value).unwrap_or(ThemeMode::System))
}

pub fn set_theme_mode(conn: &mut SqliteConnection, mode: ThemeMode) -> Result<ThemeMode, String> {
    upsert_setting(conn, THEME_KEY, mode.as_str())?;
    Ok(mode)
}

fn native_theme(theme_hint: &str) -> Option<Theme> {
    match theme_hint {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None,
    }
}

fn prefer_dark_theme(theme_hint: &str) -> bool {
    theme_hint == "dark"
}

#[cfg(target_os = "linux")]
fn apply_gtk_theme(theme_hint: &str) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(prefer_dark_theme(theme_hint));
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_gtk_theme(_theme_hint: &str) {}

pub fn apply_native_theme(app: &AppHandle, theme_hint: &str) {
    let runner = app.clone();
    let app = app.clone();
    let theme_hint = theme_hint.to_string();

    let _ = runner.run_on_main_thread(move || {
        app.set_theme(native_theme(&theme_hint));
        apply_gtk_theme(&theme_hint);
    });
}

pub fn spawn_theme_watcher(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        #[cfg(target_os = "linux")]
        {
            use zbus::message::Type;
            use zbus::blocking::{Connection, MessageIterator};
            use zbus::MatchRule;

            let Some(conn) = Connection::session().ok() else {
                eprintln!("[theme] failed to connect to session bus");
                return;
            };

            let rule = match MatchRule::builder()
                .msg_type(Type::Signal)
                .sender("org.freedesktop.portal.Desktop")
                .and_then(|rule| rule.interface("org.freedesktop.portal.Settings"))
                .and_then(|rule| rule.member("SettingChanged"))
                .and_then(|rule| rule.add_arg("org.freedesktop.appearance"))
                .and_then(|rule| rule.add_arg("color-scheme"))
            {
                Ok(rule) => rule.build(),
                Err(e) => {
                    eprintln!("[theme] failed to build match rule: {e}");
                    return;
                }
            };

            let iter = match MessageIterator::for_match_rule(rule, &conn, Some(8)) {
                Ok(iter) => iter,
                Err(e) => {
                    eprintln!("[theme] failed to subscribe to portal changes: {e}");
                    return;
                }
            };

            for message in iter {
                let Ok(message) = message else {
                    continue;
                };

                let Ok((namespace, key, value)) = message
                    .body()
                    .deserialize::<(String, String, zvariant::OwnedValue)>()
                else {
                    continue;
                };

                if namespace != "org.freedesktop.appearance" || key != "color-scheme" {
                    continue;
                }

                let theme = match u32::try_from(value).ok() {
                    Some(1) => "dark",
                    Some(2) => "light",
                    _ => "system",
                };

                let mode = {
                    let db = app.state::<DbState>();
                    let mut conn = db.0.lock().unwrap();
                    theme_mode(&mut conn).unwrap_or(ThemeMode::System)
                };

                if mode == ThemeMode::System {
                    apply_native_theme(&app, theme);
                    let _ = app.emit("theme://changed", theme);
                }
            }
        }
    });
}

#[cfg(target_os = "linux")]
fn portal_color_scheme() -> Option<u32> {
    use zbus::blocking::{Connection, Proxy};
    use zvariant::OwnedValue;

    let conn = Connection::session().ok()?;
    let proxy = Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )
    .ok()?;

    let value: OwnedValue = proxy
        .call("ReadOne", &("org.freedesktop.appearance", "color-scheme"))
        .ok()?;

    u32::try_from(value).ok()
}

pub fn theme_hint(conn: &mut SqliteConnection) -> String {
    let mode = theme_mode(conn).unwrap_or(ThemeMode::System);

    match mode {
        ThemeMode::Light => "light".to_string(),
        ThemeMode::Dark => "dark".to_string(),
        ThemeMode::System => {
            #[cfg(target_os = "linux")]
            {
                match portal_color_scheme() {
                    Some(1) => "dark".to_string(),
                    Some(2) => "light".to_string(),
                    _ => "system".to_string(),
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                "system".to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{native_theme, prefer_dark_theme, ThemeMode};

    #[test]
    fn parses_theme_modes() {
        assert_eq!(ThemeMode::parse("system"), Some(ThemeMode::System));
        assert_eq!(ThemeMode::parse("light"), Some(ThemeMode::Light));
        assert_eq!(ThemeMode::parse("dark"), Some(ThemeMode::Dark));
        assert_eq!(ThemeMode::parse("wat"), None);
    }

    #[test]
    fn maps_theme_hints_to_native_theme() {
        assert_eq!(native_theme("system"), None);
        assert_eq!(native_theme("light"), Some(tauri::Theme::Light));
        assert_eq!(native_theme("dark"), Some(tauri::Theme::Dark));
    }

    #[test]
    fn maps_theme_hints_to_gtk_preference() {
        assert!(!prefer_dark_theme("system"));
        assert!(!prefer_dark_theme("light"));
        assert!(prefer_dark_theme("dark"));
    }
}
