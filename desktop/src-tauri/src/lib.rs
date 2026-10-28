//! Drywire's desktop app: a Tauri v2 window, written in React, around the
//! same Rust core as every other client. See `desktop/README.md`.
//!
//! - [`app`]: what each command does, without the window.
//! - [`session`]: a call in progress, on a thread of its own.
//! - [`store`] and [`vault`]: the identity and the space keys on disk,
//!   protected by DPAPI on Windows.
//! - `shell.rs`: the tray icon, the global mute shortcut and the overlay.
//!
//! The interface never sits on the audio path: it polls a copy of the
//! call's counters, refreshed by the session thread.

pub mod app;
mod commands;
pub mod error;
pub mod session;
mod shell;
pub mod store;
pub mod vault;

use std::path::PathBuf;
use std::sync::Arc;

use drywire_core::audio::{AudioOptions, default_devices};
use tauri::{Emitter, Manager, WindowEvent};

use crate::app::Desktop;
use crate::session::{DeviceFactory, Notice, Notices};
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
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let store = Store::open(&data_dir(app)?)?;
            // Arrivals and departures go to every window as events: the
            // main window plays a sound, the overlay shows a pill.
            let handle = app.handle().clone();
            let notices: Notices = Arc::new(move |notice: Notice| {
                let _ = handle.emit("presence", notice);
            });
            let desktop = Desktop::new(store, Arc::new(default_sound_card), notices);
            let shortcut = desktop.settings().mute_shortcut;
            app.manage(desktop);
            shell::setup(app)?;
            // A shortcut another app holds must not stop the app: the
            // settings panel shows the error when it is changed.
            if let Err(e) = shell::register_shortcut(app.handle(), &shortcut) {
                eprintln!("drywire: the mute shortcut is not active: {e}");
            }
            Ok(())
        })
        // Closing the window hides it: the call goes on, in the tray.
        .on_window_event(|window, event| {
            if window.label() == shell::MAIN
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::public_key,
            commands::accept_invitation,
            commands::list_spaces,
            commands::join_room,
            commands::leave_room,
            commands::call_status,
            commands::set_muted,
            commands::set_gate,
            commands::set_volume,
            commands::get_settings,
            commands::set_preferences,
            commands::set_language,
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
