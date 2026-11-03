//! Ctrl-C ends `join` cleanly: the call and the session are left, so the
//! others hear this client leave at once, instead of the relay noticing
//! its silence half a minute later. A second Ctrl-C quits at once, in
//! case leaving hangs.
//!
//! The handlers only set a flag; the main loop checks it.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Whether Ctrl-C was pressed since [`install`].
pub fn requested() -> bool {
    REQUESTED.load(Ordering::Relaxed)
}

/// Catches Ctrl-C (and, on Unix, SIGTERM) from now on.
#[cfg(unix)]
pub fn install() {
    extern "C" fn handle(signal: libc::c_int) {
        REQUESTED.store(true, Ordering::Relaxed);
        // SAFETY: `signal` is async-signal-safe; the next one gets the
        // default action, which ends the process.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
    let handler = handle as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: the handler only touches an atomic and calls `signal`, both
    // allowed in a signal handler.
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

/// Catches Ctrl-C and Ctrl-Break from now on.
#[cfg(windows)]
pub fn install() {
    use windows::Win32::Foundation::{FALSE, TRUE};
    use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
    use windows::core::BOOL;

    unsafe extern "system" fn handle(kind: u32) -> BOOL {
        if kind != CTRL_C_EVENT && kind != CTRL_BREAK_EVENT {
            // Closing the console or logging off: the default, which ends
            // the process.
            return FALSE;
        }
        // The second press gets the default too.
        if REQUESTED.swap(true, Ordering::Relaxed) {
            FALSE
        } else {
            TRUE
        }
    }
    // SAFETY: the handler runs on a thread of its own and only touches an
    // atomic. Without a console there is nothing to catch, so a failure
    // leaves Ctrl-C as it was.
    let _ = unsafe { SetConsoleCtrlHandler(Some(handle), true) };
}
