// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::IsTerminal;

#[cfg(target_os = "linux")]
fn is_graphical_entrypoint() -> bool {
    std::env::var_os("APPIMAGE").is_some()
        || std::env::var_os("DESKTOP_STARTUP_ID").is_some()
        || std::env::var_os("XDG_ACTIVATION_TOKEN").is_some()
        || std::env::var_os("GIO_LAUNCHED_DESKTOP_FILE").is_some()
}

#[cfg(target_os = "linux")]
fn is_detached_graphical_session() -> bool {
    let in_graphical_session = std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
        || matches!(
            std::env::var("XDG_SESSION_TYPE").as_deref(),
            Ok("x11") | Ok("wayland")
        );

    in_graphical_session
        && std::env::var_os("TERM").is_none()
        && !std::io::stdin().is_terminal()
        && !std::io::stdout().is_terminal()
        && !std::io::stderr().is_terminal()
}

fn should_launch_gui(args: &[String]) -> bool {
    if args.get(1).map(String::as_str) == Some("app") {
        return true;
    }

    if args.len() != 1 {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        if is_graphical_entrypoint() || is_detached_graphical_session() {
            return true;
        }
    }

    false
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if should_launch_gui(&args) {
        fini_lib::run();
        return;
    }

    std::process::exit(fini_lib::run_cli());
}
