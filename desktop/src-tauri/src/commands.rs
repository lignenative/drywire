//! The commands the interface invokes. Each one calls the [`Desktop`]
//! method of the same name; the blocking ones (network, sound card) run
//! on Tauri's thread pool (`async`), never on the window's thread.

use tauri::State;

use crate::app::{Desktop, SpaceView};
use crate::error::AppError;
use crate::session::CallView;

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
    desktop: State<'_, Desktop>,
    instance: String,
    space_id: String,
    room_id: String,
) -> Result<CallView, AppError> {
    desktop.join(&instance, &space_id, &room_id)
}

/// Leaves the call.
#[tauri::command(async)]
pub fn leave_room(desktop: State<'_, Desktop>) {
    desktop.leave();
}

/// Mutes or unmutes the microphone.
#[tauri::command]
pub fn set_muted(desktop: State<'_, Desktop>, muted: bool) {
    desktop.set_muted(muted);
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
