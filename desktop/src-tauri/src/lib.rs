//! Drywire's desktop app: a Tauri v2 window, written in React, around the
//! same Rust core as every other client. See `desktop/README.md`.
//!
//! - [`app`]: what each command does, without the window.
//! - [`session`]: a call in progress, on a thread of its own.
//! - [`store`] and [`vault`]: the identity and the space keys on disk,
//!   protected by DPAPI on Windows.
//!
//! The interface never sits on the audio path: it polls a copy of the
//! call's counters, refreshed by the session thread.

pub mod app;
mod commands;
pub mod error;
pub mod session;
pub mod store;
pub mod vault;

use std::path::PathBuf;
use std::sync::Arc;

use drywire_core::audio::{AudioOptions, default_devices};
use tauri::Manager;

use crate::app::Desktop;
use crate::session::DeviceFactory;
use crate::store::Store;

/// The system's default sound card, with the smallest buffer it accepts.
fn default_sound_card() -> DeviceFactory {
    Box::new(|| default_devices(AudioOptions::default()).map_err(|e| e.to_string()))
}

/// Where the app keeps its files: `DRYWIRE_DESKTOP_HOME` if set (to run
/// two copies side by side while developing), else the system's place for
/// this app's data.
fn data_dir(app: &tauri::App) -> Result<PathBuf, String> {
    match std::env::var_os("DRYWIRE_DESKTOP_HOME") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => app.path().app_data_dir().map_err(|e| e.to_string()),
    }
}

/// Starts the app; returns when it quits.
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let store = Store::open(&data_dir(app)?)?;
            app.manage(Desktop::new(store, Arc::new(default_sound_card)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::public_key,
            commands::accept_invitation,
            commands::list_spaces,
            commands::join_room,
            commands::leave_room,
            commands::call_status,
        ])
        .build(tauri::generate_context!())
        .expect("Drywire could not start");
    app.run(|app, event| {
        // Leave the room on the way out, so the others see it at once.
        if let tauri::RunEvent::Exit = event {
            app.state::<Desktop>().leave();
        }
    });
}
