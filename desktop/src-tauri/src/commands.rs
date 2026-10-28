//! The commands the interface invokes. Each one calls the [`Desktop`]
//! method of the same name; the blocking ones (network, sound card) run
//! on Tauri's thread pool (`async`), never on the window's thread.

use tauri::{AppHandle, State};

use std::time::Duration;

use crate::app::{CalibrationView, Desktop, LoopbackView, Preferences, SpaceView};
use crate::error::AppError;
use crate::session::CallView;
use crate::shell;
use crate::store::Settings;

/// The device identity's public key, in hex.
#[tauri::command(async)]
pub fn public_key(desktop: State<'_, Desktop>) -> Result<String, AppError> {
    desktop.public_key()
}

/// Redeems an invitation link.
#[tauri::command(async)]
pub fn accept_invitation(
    desktop: State<'_, Desktop>,
    link: String,
    display_name: String,
) -> Result<SpaceView, AppError> {
    desktop.accept(&link, &display_name)
}

/// The known spaces and their rooms.
#[tauri::command(async)]
pub fn list_spaces(desktop: State<'_, Desktop>) -> Result<Vec<SpaceView>, AppError> {
    desktop.spaces()
}

/// Joins a room on the default sound card.
#[tauri::command(async)]
pub fn join_room(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    instance: String,
    space_id: String,
    room_id: String,
) -> Result<CallView, AppError> {
    let joined = desktop.join(&instance, &space_id, &room_id);
    shell::update_overlay(&app);
    shell::refresh_tray(&app, false);
    joined
}

/// Leaves the call.
#[tauri::command(async)]
pub fn leave_room(app: AppHandle, desktop: State<'_, Desktop>) {
    desktop.leave();
    shell::update_overlay(&app);
    shell::refresh_tray(&app, false);
}

/// Mutes or unmutes the microphone.
#[tauri::command]
pub fn set_muted(app: AppHandle, desktop: State<'_, Desktop>, muted: bool) {
    desktop.set_muted(muted);
    shell::refresh_tray(&app, muted);
}

/// Moves the voice gate's threshold, in dBFS.
#[tauri::command(async)]
pub fn set_gate(desktop: State<'_, Desktop>, open_dbfs: f32) -> Result<(), AppError> {
    desktop.set_gate(open_dbfs)
}

/// Plays a person at a volume from 0 to 2.
#[tauri::command(async)]
pub fn set_volume(
    desktop: State<'_, Desktop>,
    public_key: String,
    volume: f32,
) -> Result<(), AppError> {
    desktop.set_volume(&public_key, volume)
}

/// The call, as the interface shows it; polled a few times a second.
#[tauri::command]
pub fn call_status(desktop: State<'_, Desktop>) -> Option<CallView> {
    desktop.call()
}

/// The person's preferences.
#[tauri::command(async)]
pub fn get_settings(desktop: State<'_, Desktop>) -> Settings {
    desktop.settings()
}

/// Saves the settings panel: the shortcut is registered first, so one
/// that cannot be used is refused and the previous one kept.
#[tauri::command(async)]
pub fn set_preferences(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    preferences: Preferences,
) -> Result<Settings, AppError> {
    let previous = desktop.settings().mute_shortcut;
    if let Err(e) = shell::register_shortcut(&app, &preferences.mute_shortcut) {
        let _ = shell::register_shortcut(&app, &previous);
        return Err(e);
    }
    let settings = desktop.set_preferences(preferences)?;
    shell::update_overlay(&app);
    Ok(settings)
}

/// The interface's language, for the tray menu.
#[tauri::command]
pub fn set_language(app: AppHandle, language: String) {
    shell::set_language(&app, &language);
}

/// The loopback test: at most 8 seconds.
#[tauri::command(async)]
pub fn loopback_test(desktop: State<'_, Desktop>) -> Result<LoopbackView, AppError> {
    desktop.loopback_test(Duration::from_secs(8))
}

/// The gate calibration: 10 seconds, controller in hand (spec section 10).
#[tauri::command(async)]
pub fn calibrate_gate(desktop: State<'_, Desktop>) -> Result<CalibrationView, AppError> {
    desktop.calibrate(Duration::from_secs(10))
}
