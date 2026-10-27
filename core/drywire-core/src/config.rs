//! What a client needs to take part in a call: its session on the relay,
//! the room's keys, its identity, and who else is in the room.
//!
//! All of it comes from the control plane when the client joins a room
//! (protocol section 13.6), except the space key, which the client got from
//! the invitation link and the server never sees, and the identity, which
//! the client keeps. Members who join later are added to the running call
//! with [`Call::add_member`](crate::call::Call::add_member).

use std::fmt;

use drywire_proto::control::SECRET_SIZE;
use drywire_proto::crypto::{KEY_SIZE, ROOM_ID_SIZE, SigningKey, VerifyingKey};

/// Everything a call starts from. Its `Debug` output leaves out the
/// secrets, so a log line or a `dbg!` cannot leak them.
#[derive(Clone)]
pub struct ClientConfig {
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
    /// Everyone in the room when the call starts, this client included.
    pub members: Vec<Member>,
}

impl fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientConfig")
            .field("room_id", &to_hex(&self.room_id))
            .field("space_key", &"<secret>")
            .field("session", &self.session)
            .field("session_secret", &"<secret>")
            .field("identity", &"<secret>")
            .field("members", &self.members)
            .finish()
    }
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

impl ClientConfig {
    /// This client's own entry in the member list, if it is there.
    pub fn me(&self) -> Option<&Member> {
        self.member(self.session)
    }

    /// The member with this session, if any.
    pub fn member(&self, session: u32) -> Option<&Member> {
        self.members.iter().find(|m| m.session == session)
    }
}

/// Lowercase hexadecimal.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_the_secrets() {
        let identity = SigningKey::from_bytes(&[0xa1; 32]);
        let config = ClientConfig {
            room_id: [0x0f; 16],
            space_key: [0x5a; 32],
            session: 1,
            session_secret: [0x11; 32],
            members: vec![Member {
                session: 1,
                name: "alex".into(),
                public_key: identity.verifying_key(),
            }],
            identity,
        };
        let shown = format!("{config:?}");
        assert!(shown.contains("alex"), "{shown}");
        for secret in ["5a5a", "1111", "a1a1"] {
            assert!(!shown.contains(secret), "{secret} leaked: {shown}");
        }
        assert_eq!(config.me().map(|m| m.name.as_str()), Some("alex"));
    }
}
