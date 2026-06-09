// Prevents an extra console window for the desktop GUI on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    fini_lib::run();
}
