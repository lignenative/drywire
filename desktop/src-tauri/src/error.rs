//! Errors as the interface receives them: a stable code it can translate,
//! and the detail in English, shown under the translated sentence.

use serde::Serialize;

/// Why a command failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppError {
    /// What kind of failure, for the interface to translate:
    /// `storage`, `invalid_link`, `bad_instance`, `refused`, `network`,
    /// `unavailable`, `protocol`, `relay`, `audio`, `not_found`,
    /// `shortcut`, `busy`, `autostart`.
    pub code: &'static str,
    /// The detail, in English.
    pub message: String,
}

impl AppError {
    /// An error with this code.
    pub fn new(code: &'static str, message: impl Into<String>) -> AppError {
        AppError {
            code,
            message: message.into(),
        }
    }

    /// Whether trying again later may succeed: the server or the relay
    /// could not be reached, or was busy.
    pub fn is_temporary(&self) -> bool {
        matches!(self.code, "network" | "relay" | "unavailable")
    }

    /// The app's own files could not be read or written.
    pub fn storage(message: impl Into<String>) -> AppError {
        AppError::new("storage", message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<drywire_control::Error> for AppError {
    fn from(e: drywire_control::Error) -> AppError {
        let code = match &e {
            // Restarting, overloaded, or a proxy with nothing behind it.
            drywire_control::Error::Refused { status, .. } if *status >= 500 || *status == 429 => {
                "unavailable"
            }
            drywire_control::Error::Refused { .. } => "refused",
            drywire_control::Error::Network(_) => "network",
            drywire_control::Error::Protocol(_) => "protocol",
            drywire_control::Error::BadInstance(_) => "bad_instance",
            drywire_control::Error::BadCode(_) => "bad_code",
        };
        AppError::new(code, e.to_string())
    }
}
