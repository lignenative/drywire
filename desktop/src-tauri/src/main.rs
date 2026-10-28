//! The `drywire-desktop` binary: see the library.

// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    drywire_desktop::run();
}
