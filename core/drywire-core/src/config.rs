//! The client configuration file of phase 1 (ADR 0006).
//!
//! Until the control plane exists, everything a client would learn when
//! joining a room comes from one JSON file: where the relay is, the client's
//! session and its secret, the keys of the space, and who the other members
//! are. A file looks like this (hex strings shortened):
//!
//! ```json
//! {
//!   "dev_only": true,
//!   "relay": "127.0.0.1:10050",
//!   "room_id": "0f0e…",
//!   "space_key": "5ace…",
//!   "session": 1,
//!   "session_secret": "1111…",
//!   "identity_seed": "a1a1…",
//!   "members": [
//!     { "session": 1, "name": "alex", "public_key": "…" },
//!     { "session": 2, "name": "sam", "public_key": "…" }
//!   ]
//! }
//! ```
//!
//! Every member of a room gets the same `room_id`, `space_key` and `members`
//! list, and its own `session`, `session_secret` and `identity_seed`. The
//! file holds secrets: treat it like a password. The committed development
//! files say `"dev_only": true`, and [`ClientConfig::check_relay`] refuses
//! to use them with a relay that is not on this machine, as the server does
//! with its development rooms (ADR 0004).

use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;

use drywire_proto::control::SECRET_SIZE;
use drywire_proto::crypto::{KEY_SIZE, ROOM_ID_SIZE, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// A parsed and checked client configuration.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Whether the file's secrets are public (committed to the repository).
    pub dev_only: bool,
    /// The relay's UDP address, as written: a host name or an IP, and a port.
    pub relay: String,
    /// The room's identifier, part of the key derivation.
    pub room_id: [u8; ROOM_ID_SIZE],
    /// The secret shared by the members of the space.
    pub space_key: [u8; KEY_SIZE],
    /// This client's session.
    pub session: u32,
    /// The secret that signs this client's `hello` packets.
    pub session_secret: [u8; SECRET_SIZE],
    /// This client's identity, which signs its announcements.
    pub identity: SigningKey,
    /// Every member of the room, this client included.
    pub members: Vec<Member>,
}

/// One member of the room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The member's session.
    pub session: u32,
    /// A display name; only shown, never sent.
    pub name: String,
    /// The key that verifies the member's announcements.
    pub public_key: VerifyingKey,
}

/// Why a configuration file was refused.
#[derive(Debug)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn error(message: impl Into<String>) -> ConfigError {
    ConfigError(message.into())
}

/// The file as written on disk, before checks: every key is hex.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    dev_only: bool,
    relay: String,
    room_id: String,
    space_key: String,
    session: u32,
    session_secret: String,
    identity_seed: String,
    members: Vec<FileMember>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMember {
    session: u32,
    name: String,
    public_key: String,
}

impl ClientConfig {
    /// Reads and checks a configuration file.
    pub fn load(path: &Path) -> Result<ClientConfig, ConfigError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| error(format!("{}: {e}", path.display())))?;
        ClientConfig::parse(&text).map_err(|e| error(format!("{}: {e}", path.display())))
    }

    /// Parses and checks the JSON text of a configuration file.
    pub fn parse(text: &str) -> Result<ClientConfig, ConfigError> {
        let file: File = serde_json::from_str(text).map_err(|e| error(e.to_string()))?;
        let identity = SigningKey::from_bytes(&hex_array(&file.identity_seed, "identity_seed")?);
        let mut members = Vec::with_capacity(file.members.len());
        for m in &file.members {
            let key = hex_array(&m.public_key, "public_key")?;
            let public_key = VerifyingKey::from_bytes(&key)
                .map_err(|_| error(format!("session {}: invalid public key", m.session)))?;
            members.push(Member {
                session: m.session,
                name: m.name.clone(),
                public_key,
            });
        }
        let config = ClientConfig {
            dev_only: file.dev_only,
            relay: file.relay,
            room_id: hex_array(&file.room_id, "room_id")?,
            space_key: hex_array(&file.space_key, "space_key")?,
            session: file.session,
            session_secret: hex_array(&file.session_secret, "session_secret")?,
            identity,
            members,
        };
        config.validate()?;
        Ok(config)
    }

    /// The JSON text of this configuration, as [`ClientConfig::parse`] reads it.
    pub fn to_json(&self) -> String {
        let file = File {
            dev_only: self.dev_only,
            relay: self.relay.clone(),
            room_id: to_hex(&self.room_id),
            space_key: to_hex(&self.space_key),
            session: self.session,
            session_secret: to_hex(&self.session_secret),
            identity_seed: to_hex(&self.identity.to_bytes()),
            members: self
                .members
                .iter()
                .map(|m| FileMember {
                    session: m.session,
                    name: m.name.clone(),
                    public_key: to_hex(m.public_key.as_bytes()),
                })
                .collect(),
        };
        serde_json::to_string_pretty(&file).expect("a config always serializes") + "\n"
    }

    /// Sessions are non-zero and unique, and this client is one of the
    /// members, with the public key of its own identity: a file copied from
    /// another member, or edited by hand, is caught here rather than as
    /// audio nobody can decrypt.
    fn validate(&self) -> Result<(), ConfigError> {
        let mut seen = std::collections::HashSet::new();
        for m in &self.members {
            if m.session == 0 {
                return Err(error("session 0 is not valid"));
            }
            if !seen.insert(m.session) {
                return Err(error(format!("session {} is listed twice", m.session)));
            }
        }
        let me = self
            .members
            .iter()
            .find(|m| m.session == self.session)
            .ok_or_else(|| error(format!("session {} is not in members", self.session)))?;
        if me.public_key != self.identity.verifying_key() {
            return Err(error(format!(
                "the public key of session {} does not match identity_seed",
                self.session
            )));
        }
        Ok(())
    }

    /// This client's own entry in the member list.
    pub fn me(&self) -> &Member {
        self.member(self.session)
            .expect("validate checked that this client is a member")
    }

    /// The member with this session, if any.
    pub fn member(&self, session: u32) -> Option<&Member> {
        self.members.iter().find(|m| m.session == session)
    }

    /// Resolves `relay` (or `relay_override`) to an address, and refuses a
    /// development file with a relay on another machine.
    pub fn check_relay(&self, relay_override: Option<&str>) -> Result<SocketAddr, ConfigError> {
        let relay = relay_override.unwrap_or(&self.relay);
        let addr = relay
            .to_socket_addrs()
            .map_err(|e| error(format!("relay {relay}: {e}")))?
            .next()
            .ok_or_else(|| error(format!("relay {relay}: no address")))?;
        if self.dev_only && !addr.ip().is_loopback() {
            return Err(error(format!(
                "this configuration holds public development keys and may only be used \
                 with a relay on this machine, not {relay}"
            )));
        }
        Ok(addr)
    }
}

/// Lowercase hexadecimal.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes hexadecimal of exactly `N` bytes.
fn hex_array<const N: usize>(text: &str, field: &str) -> Result<[u8; N], ConfigError> {
    let bad = || error(format!("{field} must be {} hex digits", 2 * N));
    if text.len() != 2 * N || !text.is_ascii() {
        return Err(bad());
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ClientConfig {
        let identity = SigningKey::from_bytes(&[0xa1; 32]);
        let other = SigningKey::from_bytes(&[0xa2; 32]);
        ClientConfig {
            dev_only: true,
            relay: "127.0.0.1:10050".into(),
            room_id: [0x0f; 16],
            space_key: [0x5a; 32],
            session: 1,
            session_secret: [0x11; 32],
            members: vec![
                Member {
                    session: 1,
                    name: "alex".into(),
                    public_key: identity.verifying_key(),
                },
                Member {
                    session: 2,
                    name: "sam".into(),
                    public_key: other.verifying_key(),
                },
            ],
            identity,
        }
    }

    #[test]
    fn a_config_round_trips_through_json() {
        let config = sample();
        let back = ClientConfig::parse(&config.to_json()).unwrap();
        assert_eq!(back.to_json(), config.to_json());
        assert_eq!(back.me().name, "alex");
    }

    #[test]
    fn inconsistent_files_are_refused() {
        let mut wrong_key = sample();
        wrong_key.session = 2; // sam's entry, alex's seed
        let mut missing = sample();
        missing.session = 3;
        let mut twice = sample();
        twice.members[1].session = 1;

        for (config, message) in [
            (wrong_key, "does not match"),
            (missing, "not in members"),
            (twice, "listed twice"),
        ] {
            let err = ClientConfig::parse(&config.to_json()).unwrap_err();
            assert!(err.to_string().contains(message), "{err}");
        }
        let short = sample().to_json().replace(&"0f".repeat(16), "0f");
        assert!(ClientConfig::parse(&short).is_err());
    }

    #[test]
    fn a_dev_config_only_talks_to_this_machine() {
        let config = sample();
        assert!(config.check_relay(None).is_ok());
        assert!(config.check_relay(Some("[::1]:10050")).is_ok());
        let err = config.check_relay(Some("192.0.2.1:10050")).unwrap_err();
        assert!(err.to_string().contains("development"), "{err}");

        let real = ClientConfig {
            dev_only: false,
            ..sample()
        };
        assert!(real.check_relay(Some("192.0.2.1:10050")).is_ok());
    }
}
