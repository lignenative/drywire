//! What lives outside the main window, so a player in a match never has
//! to bring it back (spec section 10, "Presence" and "Windows and
//! Linux"):
//!
//! - **The tray icon**: show the window, mute or unmute, quit. Closing the
//!   window only hides it; the call goes on.
//! - **The global mute shortcut**, heard while a game has the focus.
//! - **The overlay**: a transparent window, always on top, that lets
//!   every click through to the game. It shows who is speaking, whether
//!   this device is muted, and a pill for two seconds when someone joins
//!   or leaves. It needs the game in borderless windowed mode; nothing is
//!   injected into the game.
//!
//! The presence sounds are played by the main window's interface, from
//! the `presence` events the session sends (`lib.rs`): through the web
//! view's own audio output, never mixed into the call.

use std::sync::Mutex;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::app::Desktop;
use crate::error::AppError;

/// The window that shows who speaks over the game.
pub const OVERLAY: &str = "overlay";
/// The app's window.
pub const MAIN: &str = "main";

/// The tray menu's entries, kept to change their text.
pub struct Tray {
    show: MenuItem<Wry>,
    mute: MenuItem<Wry>,
    quit: MenuItem<Wry>,
    /// The interface's language, `en` or `fr`, for the menu.
    language: Mutex<String>,
}

/// The tray menu's texts, in the interface's language.
fn labels(language: &str, muted: bool) -> [&'static str; 3] {
    match (language, muted) {
        ("fr", false) => ["Afficher Drywire", "Couper le micro", "Quitter"],
        ("fr", true) => ["Afficher Drywire", "Réactiver le micro", "Quitter"],
        (_, false) => ["Show Drywire", "Mute", "Quit"],
        (_, true) => ["Show Drywire", "Unmute", "Quit"],
    }
}

/// Puts the icon in the tray, and makes the overlay let clicks through.
pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let [show_text, mute_text, quit_text] = labels("en", false);
    let show = MenuItem::with_id(app, "show", show_text, true, None::<&str>)?;
    let mute = MenuItem::with_id(app, "mute", mute_text, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_text, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &mute, &separator, &quit])?;
    let mut tray = TrayIconBuilder::with_id("drywire")
        .tooltip("Drywire")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "mute" => toggle_mute(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(Tray {
        show,
        mute,
        quit,
        language: Mutex::new("en".into()),
    });

    if let Some(overlay) = app.get_webview_window(OVERLAY) {
        overlay.set_ignore_cursor_events(true)?;
    }
    Ok(())
}

/// Brings the main window back.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// What the global shortcut and the tray's entry do.
pub fn toggle_mute(app: &AppHandle) {
    if let Some(muted) = app.state::<Desktop>().toggle_muted() {
        refresh_tray(app, muted);
    }
}

/// Writes the tray menu in `language` (`en` or `fr`).
pub fn set_language(app: &AppHandle, language: &str) {
    let tray = app.state::<Tray>();
    *tray.language.lock().expect("not poisoned") = language.to_string();
    let muted = app.state::<Desktop>().call().is_some_and(|call| call.muted);
    refresh_tray(app, muted);
}

/// Says "Mute" or "Unmute" in the tray, in the interface's language.
pub fn refresh_tray(app: &AppHandle, muted: bool) {
    let tray = app.state::<Tray>();
    let language = tray.language.lock().expect("not poisoned").clone();
    let [show, mute, quit] = labels(&language, muted);
    let _ = tray.show.set_text(show);
    let _ = tray.mute.set_text(mute);
    let _ = tray.quit.set_text(quit);
}

/// Listens for the global mute shortcut, replacing the previous one. An
/// empty one turns it off. Fails when the system or another app holds
/// that shortcut, or it cannot be read.
pub fn register_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), AppError> {
    let shortcuts = app.global_shortcut();
    shortcuts
        .unregister_all()
        .map_err(|e| AppError::new("shortcut", e.to_string()))?;
    if shortcut.trim().is_empty() {
        return Ok(());
    }
    shortcuts
        .on_shortcut(shortcut.trim(), |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_mute(app);
            }
        })
        .map_err(|e| AppError::new("shortcut", format!("{shortcut}: {e}")))
}

/// Shows the overlay during a call when the person wants it, and hides it
/// otherwise. It never takes the focus.
pub fn update_overlay(app: &AppHandle) {
    let Some(overlay) = app.get_webview_window(OVERLAY) else {
        return;
    };
    let desktop = app.state::<Desktop>();
    let wanted = desktop.settings().overlay && desktop.call().is_some();
    let _ = if wanted {
        overlay.show()
    } else {
        overlay.hide()
    };
}

#[cfg(test)]
mod tests {
    use super::labels;

    #[test]
    fn the_tray_speaks_both_languages() {
        assert_eq!(labels("fr", true)[1], "Réactiver le micro");
        assert_eq!(labels("en", false)[1], "Mute");
        assert_eq!(labels("de", false), labels("en", false));
    }
}
