//! A space's MLS group (RFC 9420, ADR 0052).
//!
//! Every device of every member is a leaf of the space's group. The group
//! hands each epoch's secret to its current leaves only, and the room keys
//! of the audio path come from it through MLS's exporter. Adding or
//! removing a device starts a new epoch: a removed device learns nothing
//! after, and a leaked epoch secret opens nothing before.
//!
//! This crate keeps the group's state and makes or reads its messages. It
//! does no I/O: the apps carry the bytes to and from the control plane's
//! delivery service (protocol section 16), which orders them.
//!
//! - A [`Device`] is this device's MLS identity: an Ed25519 key of its own,
//!   certified by the account key (see [`credential`]), and the state of
//!   every group it is in. [`Device::save`] gives it all as bytes for the
//!   device's sealed store.
//! - A [`Group`] is one space's group, as this device sees it.

pub mod credential;
mod device;
mod group;

pub use device::Device;
pub use group::{Commit, Group, Leaf, PendingDevice};

/// The one cipher suite Drywire's groups use: X25519, AES-128-GCM,
/// SHA-256 and Ed25519, the suite every MLS implementation must offer.
pub const CIPHERSUITE: openmls::prelude::Ciphersuite =
    openmls::prelude::Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// What went wrong with a group.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Bytes that are not the expected MLS message.
    Malformed,
    /// A leaf whose credential is not certified by the account it names.
    BadCredential,
    /// A key package for another device than the one it was posted for,
    /// or for another account.
    WrongDevice,
    /// A leaf for an account that is not a member of the space.
    NotMember,
    /// Saved state this version cannot read.
    BadState,
    /// OpenMLS refused the operation; the text says why.
    Mls(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Malformed => f.write_str("not the expected MLS message"),
            Error::BadCredential => f.write_str("a device is not certified by its account"),
            Error::WrongDevice => {
                f.write_str("a key package is not the device's it was posted for")
            }
            Error::NotMember => f.write_str("a device belongs to someone who is not a member"),
            Error::BadState => f.write_str("saved group state this version cannot read"),
            Error::Mls(e) => write!(f, "MLS: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Turns any OpenMLS error into [`Error::Mls`].
fn mls(e: impl std::fmt::Display) -> Error {
    Error::Mls(e.to_string())
}
