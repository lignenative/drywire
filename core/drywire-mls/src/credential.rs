//! A leaf's credential: which device it is (ADR 0052, as ADR 0055 made
//! it).
//!
//! Each device has an Ed25519 key of its own, the one it logs in with
//! (ADR 0055), and that key signs for it in every group: two leaves never
//! share it, which MLS requires. Its basic credential's identity is that
//! same public key. A member accepts a leaf only if its identity is its
//! signature key and the control plane lists that key among a member's
//! devices.

/// The device a leaf is, if its credential's identity is its own
/// signature key.
pub fn device(identity: &[u8], signature_key: &[u8]) -> Option<[u8; 32]> {
    let key: [u8; 32] = signature_key.try_into().ok()?;
    (identity == key).then_some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leaf_names_its_own_key_only() {
        assert_eq!(device(&[1; 32], &[1; 32]), Some([1; 32]));
        assert_eq!(device(&[2; 32], &[1; 32]), None, "another device's name");
        assert_eq!(device(&[1; 31], &[1; 31]), None, "not a key");
    }
}
