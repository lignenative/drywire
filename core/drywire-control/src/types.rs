//! What the control plane sends and receives (protocol section 13), as
//! Rust values. Keys, ids and secrets travel as lowercase hex.
//!
//! Unknown fields are ignored, so a newer server can add some without
//! breaking an older client (protocol section 11).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `N` bytes written as lowercase hex in JSON.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hex<const N: usize>(pub [u8; N]);

impl<const N: usize> Hex<N> {
    /// The hex text.
    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }

    /// Reads exactly `N` bytes of hex.
    pub fn parse(text: &str) -> Option<Hex<N>> {
        from_hex(text)?.try_into().ok().map(Hex)
    }
}

impl<const N: usize> fmt::Debug for Hex<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl<const N: usize> fmt::Display for Hex<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl<const N: usize> Serialize for Hex<N> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de, const N: usize> Deserialize<'de> for Hex<N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Hex::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("expected {} hex digits", 2 * N)))
    }
}

/// Lowercase hexadecimal.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes hexadecimal; `None` if it is not.
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// A space, room or invitation id.
pub type Id = Hex<16>;
/// An Ed25519 public key.
pub type PublicKey = Hex<32>;

/// A space.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Space {
    /// Its id.
    pub id: Id,
    /// Its name.
    pub name: String,
    /// How many members it may have.
    pub max_members: u32,
}

/// A room of a space. Its id is the `room_id` of the key derivation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Room {
    /// Its id.
    pub id: Id,
    /// Its space.
    pub space_id: Id,
    /// Its name.
    pub name: String,
}

/// A member of a space.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Member {
    /// Their identity's public key.
    pub public_key: PublicKey,
    /// Their display name.
    pub name: String,
    /// `owner`, `admin` or `member`.
    pub role: String,
}

/// A space as one of its members sees it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SpaceView {
    /// The space.
    pub space: Space,
    /// The caller's role.
    pub role: String,
    /// Its rooms, by name.
    pub rooms: Vec<Room>,
    /// Its members, owner first.
    #[serde(default)]
    pub members: Vec<Member>,
}

/// One of the caller's spaces.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Membership {
    /// The space.
    pub space: Space,
    /// The caller's role in it.
    pub role: String,
}

/// An identity as the server knows it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Identity {
    /// The public key.
    pub public_key: PublicKey,
    /// The display name.
    pub name: String,
}

/// A new invitation: the code is in this answer only.
#[derive(Clone, Deserialize)]
pub struct NewInvitation {
    /// The code to put in the link.
    pub code: Hex<16>,
    /// The instance's canonical origin, as the server knows it.
    pub instance: String,
    /// When it expires and how many uses it has.
    pub invitation: InvitationInfo,
}

impl fmt::Debug for NewInvitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewInvitation")
            .field("instance", &self.instance)
            .field("invitation", &self.invitation)
            .finish_non_exhaustive()
    }
}

/// An invitation without its code.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct InvitationInfo {
    /// Its id, to revoke it.
    pub id: Id,
    /// When it stops working, RFC 3339.
    pub expires_at: String,
    /// How many times it can be used.
    pub max_uses: u32,
    /// How many times it was used.
    pub uses: u32,
}

/// Someone in a room.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Presence {
    /// Their UDP session.
    pub session: u32,
    /// The room.
    pub room_id: Id,
    /// Their identity, which verifies their announcements.
    pub public_key: PublicKey,
    /// Their display name; empty in `leave` and `disconnect` events.
    #[serde(default)]
    pub name: String,
}

/// What joining a room gives (protocol section 13.6).
#[derive(Clone, Deserialize)]
pub struct JoinAnswer {
    /// This client's UDP session.
    pub session: u32,
    /// The secret that signs its `hello` packets.
    pub session_secret: Hex<32>,
    /// The relay's UDP address, `host:port`.
    pub relay: String,
    /// The room.
    pub room: Room,
    /// Everyone in the room, this client included.
    pub sessions: Vec<Presence>,
}

/// Leaves the session secret out.
impl fmt::Debug for JoinAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinAnswer")
            .field("session", &self.session)
            .field("relay", &self.relay)
            .field("room", &self.room)
            .field("sessions", &self.sessions)
            .finish_non_exhaustive()
    }
}

/// One event of a space's stream (protocol section 13.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Everyone in the space's rooms when the stream started.
    Snapshot(Vec<Presence>),
    /// Someone entered a room.
    Join(Presence),
    /// Someone left a room, was removed, or moved to another.
    Leave(Presence),
    /// The relay stopped hearing from someone.
    Disconnect(Presence),
    /// An event this client does not know, from a newer server.
    Other(String),
}
