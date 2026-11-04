//! The commands the interface invokes. Each one calls the [`Desktop`]
//! method of the same name; the blocking ones (network, sound card) run
//! on Tauri's thread pool (`async`), never on the window's thread.

use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use std::time::Duration;

use crate::app::{
    AccountView, CalibrationView, Desktop, DeviceCodeView, LoopbackView, Preferences, RestoredView,
    SpaceView,
};
use crate::error::AppError;
use crate::session::{CallView, METER_FLOOR_DBFS};
use crate::shell;
use crate::store::Settings;

/// The device identity's public key, in hex.
#[tauri::command(async)]
pub fn public_key(desktop: State<'_, Desktop>) -> Result<String, AppError> {
    desktop.public_key()
}

/// The account, for the settings.
#[tauri::command(async)]
pub fn account(desktop: State<'_, Desktop>) -> Result<AccountView, AppError> {
    desktop.account()
}

/// A new recovery code, to show once.
#[tauri::command(async)]
pub fn new_recovery_code(desktop: State<'_, Desktop>) -> Result<String, AppError> {
    desktop.new_recovery_code()
}

/// A device code, valid ten minutes.
#[tauri::command(async)]
pub fn new_device_code(desktop: State<'_, Desktop>) -> Result<DeviceCodeView, AppError> {
    desktop.new_device_code()
}

/// Becomes the account a code opens; returns its name and spaces.
#[tauri::command(async)]
pub fn restore_account(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    instance: String,
    code: String,
    replace: bool,
) -> Result<RestoredView, AppError> {
    let spaces = desktop.restore_account(&instance, &code, replace);
    shell::update_overlay(&app);
    shell::refresh_tray(&app, false);
    spaces
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

/// While reconnecting, joins again now instead of after the pause.
#[tauri::command]
pub fn retry_now(desktop: State<'_, Desktop>) {
    desktop.retry_now();
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

/// The microphone's level, in dBFS: the call's in a call, else from a
/// meter that opens the sound card while the interface keeps asking.
/// A hidden or minimized window closes the meter: its page may go on
/// asking (a hidden web view's timers slow down, but need not stop).
#[tauri::command(async)]
pub fn input_level(app: AppHandle, desktop: State<'_, Desktop>) -> Result<f32, AppError> {
    if !shell::main_on_screen(&app) {
        desktop.close_meter();
        return Ok(METER_FLOOR_DBFS);
    }
    desktop.input_level()
}

/// Whether the app starts with the session, hidden in the tray.
#[tauri::command]
pub fn launch_at_login(app: AppHandle) -> Result<bool, AppError> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| AppError::new("autostart", e.to_string()))
}

/// Starts the app with the session, or stops doing so. Off until the
/// person turns it on.
#[tauri::command]
pub fn set_launch_at_login(app: AppHandle, enabled: bool) -> Result<bool, AppError> {
    let manager = app.autolaunch();
    let done = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    done.map_err(|e| AppError::new("autostart", e.to_string()))?;
    launch_at_login(app)
}
