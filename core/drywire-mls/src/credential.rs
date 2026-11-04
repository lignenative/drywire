//! A device's credential: which account it belongs to, proved by that
//! account (ADR 0052).
//!
//! Two leaves of a group cannot share a signature key, so each device signs
//! in the group with an Ed25519 key of its own. Its basic credential's
//! identity says whose device it is:
//!
//! ```text
//! identity = account public key (32) ‖ signature (64)
//! signature = Ed25519(account key, "drywire/mls-device/v1" ‖ device public key (32))
//! ```
//!
//! A member accepts a leaf only if the signature verifies against the
//! account key it names, for the leaf's own signature key.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// What the account key signs to certify a device.
const LABEL: &[u8] = b"drywire/mls-device/v1";

/// The length of a credential's identity.
pub const IDENTITY_LEN: usize = 32 + 64;

/// The identity that certifies `device` as a device of `account`.
pub fn certify(account: &SigningKey, device: &[u8; 32]) -> [u8; IDENTITY_LEN] {
    let signature = account.sign(&message(device));
    let mut out = [0; IDENTITY_LEN];
    out[..32].copy_from_slice(account.verifying_key().as_bytes());
    out[32..].copy_from_slice(&signature.to_bytes());
    out
}

/// The account a credential's identity names, if it certifies the leaf
/// whose signature key is `device`.
pub fn verify(identity: &[u8], device: &[u8]) -> Option<[u8; 32]> {
    let device: &[u8; 32] = device.try_into().ok()?;
    let (account, signature) = identity.split_at_checked(32)?;
    let account: [u8; 32] = account.try_into().ok()?;
    let signature = Signature::from_slice(signature).ok()?;
    let key = VerifyingKey::from_bytes(&account).ok()?;
    key.verify(&message(device), &signature).ok()?;
    Some(account)
}

fn message(device: &[u8; 32]) -> Vec<u8> {
    [LABEL, device.as_slice()].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certificate_holds_for_its_device_only() {
        let account = SigningKey::from_bytes(&[7; 32]);
        let identity = certify(&account, &[1; 32]);
        assert_eq!(
            verify(&identity, &[1; 32]),
            Some(*account.verifying_key().as_bytes())
        );
        assert_eq!(verify(&identity, &[2; 32]), None, "another device");
        let mut forged = identity;
        forged[..32].copy_from_slice(SigningKey::from_bytes(&[8; 32]).verifying_key().as_bytes());
        assert_eq!(verify(&forged, &[1; 32]), None, "another account");
        assert_eq!(verify(&identity[..40], &[1; 32]), None, "too short");
    }
}
