//! Whether end-to-end encryption is verified for everyone heard in a call:
//! what the apps' lock shows (ADR 0041).
//!
//! [`Inbound`](crate::inbound::Inbound) counts, per member, what it checked
//! of their packets ([`SenderCheck`]). An [`EncryptionMonitor`] turns those
//! counters into one state per sender and one for the room, remembering
//! when each sender last failed, so a failure shows for
//! [`FAILURE_HOLD_US`] and then clears once their packets check again.
//!
//! The call's network thread runs it when it publishes its statistics,
//! never the audio callbacks (ADR 0017). Nothing here holds or prints a
//! key: only the public keys the control plane handed out, and counters.

use std::collections::HashMap;

use drywire_proto::crypto::VerifyingKey;

use crate::inbound::SenderCheck;

/// How long a signature or authentication failure keeps a sender, and the
/// room, `Failed`: long enough to be seen, after which a sender whose
/// packets verify again is verified again.
pub const FAILURE_HOLD_US: u64 = 10_000_000;

/// The room's state, for the lock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncryptionState {
    /// Nobody verified yet (alone, or nobody heard so far), or someone is
    /// heard whose signed announcement has not arrived: their audio is
    /// dropped until it does, at most a second later.
    #[default]
    Pending,
    /// Every sender heard is verified, and at least one is: everything
    /// played comes from keys their owners signed, and authenticated.
    Verified,
    /// In the last [`FAILURE_HOLD_US`], someone's announcement failed its
    /// signature or their audio failed authentication. Those packets were
    /// dropped, never played.
    Failed,
}

/// One sender's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SenderState {
    /// Nothing heard from them and no announcement verified: silent since
    /// this device joined. They do not hold the room back.
    Silent,
    /// Their audio arrives, but no announcement of theirs verified yet.
    Waiting,
    /// An announcement of theirs verified with the public key the control
    /// plane gave, and nothing of theirs failed lately.
    Verified,
    /// A packet in their name failed its signature or authentication in
    /// the last [`FAILURE_HOLD_US`].
    Failed,
}

/// What the lock knows of one sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SenderEncryption {
    /// Their session.
    pub session: u32,
    /// The public key their announcements verify with.
    pub public_key: VerifyingKey,
    /// Their state.
    pub state: SenderState,
    /// Their announcements whose signature failed, over the call.
    pub bad_signature: u64,
    /// Their audio packets that failed authentication, over the call.
    pub bad_tag: u64,
}

/// The lock's data: the room's state and each sender's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Encryption {
    /// The room's state.
    pub state: EncryptionState,
    /// Every other member of the room, by session.
    pub senders: Vec<SenderEncryption>,
}

/// Derives [`Encryption`] from the receiver's counters over time.
#[derive(Debug, Default)]
pub struct EncryptionMonitor {
    /// Per session: the failures last seen, and when they last grew.
    failures: HashMap<u32, (u64, u64)>,
}

impl EncryptionMonitor {
    /// A monitor that has seen nothing yet.
    pub fn new() -> EncryptionMonitor {
        EncryptionMonitor::default()
    }

    /// Updates `out` from the receiver's counters at `now_us` (any clock
    /// that only goes forward, the same at every call). Senders not in
    /// `senders` left, and are forgotten.
    pub fn update(
        &mut self,
        now_us: u64,
        senders: impl IntoIterator<Item = SenderCheck>,
        out: &mut Encryption,
    ) {
        out.senders.clear();
        for check in senders {
            let failures = check.failures();
            let failed_at = match self.failures.get(&check.session) {
                Some(&(seen, at)) if seen == failures => (seen > 0).then_some(at),
                _ => (failures > 0).then_some(now_us),
            };
            if let Some(at) = failed_at {
                self.failures.insert(check.session, (failures, at));
            }
            let recent = failed_at.is_some_and(|at| now_us.saturating_sub(at) < FAILURE_HOLD_US);
            out.senders.push(SenderEncryption {
                session: check.session,
                public_key: check.public_key,
                state: if recent {
                    SenderState::Failed
                } else if check.verified {
                    SenderState::Verified
                } else if check.heard {
                    SenderState::Waiting
                } else {
                    SenderState::Silent
                },
                bad_signature: check.bad_signature,
                bad_tag: check.bad_tag,
            });
        }
        out.senders.sort_by_key(|s| s.session);
        let present = &out.senders;
        self.failures
            .retain(|session, _| present.iter().any(|s| s.session == *session));
        out.state = room_state(&out.senders);
    }
}

/// The room's state from its senders' (see [`EncryptionState`]).
pub fn room_state(senders: &[SenderEncryption]) -> EncryptionState {
    let any = |state| senders.iter().any(|s| s.state == state);
    if any(SenderState::Failed) {
        EncryptionState::Failed
    } else if any(SenderState::Waiting) || !any(SenderState::Verified) {
        EncryptionState::Pending
    } else {
        EncryptionState::Verified
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbound::Inbound;
    use crate::opus::FrameSize;
    use crate::sender::{Packets, Sender, SenderConfig};
    use drywire_proto::crypto::SigningKey;

    const ROOM: [u8; 16] = [2; 16];
    const SPACE: [u8; 32] = [3; 32];
    const ALEX: u32 = 7;
    const SAM: u32 = 8;

    fn identity(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// A sender for `session` that signs with `seed`'s key.
    fn sender(seed: u8, session: u32) -> Sender {
        let config = SenderConfig::new(FrameSize::Ms5);
        Sender::new(identity(seed), session, ROOM, &SPACE, config).unwrap()
    }

    /// One loud frame: an announcement, then an audio packet.
    fn loud_packets(s: &mut Sender) -> Vec<Vec<u8>> {
        let pcm: Vec<f32> = (0..240).map(|i| 0.3 * (i as f32 * 0.1).sin()).collect();
        let mut out = Packets::new();
        s.process(&pcm, &mut out).unwrap();
        out.iter().map(<[u8]>::to_vec).collect()
    }

    /// A receiver with alex (key 1) and sam (key 2) in the room.
    fn room() -> (Inbound, EncryptionMonitor, Encryption) {
        let members = [
            (ALEX, identity(1).verifying_key()),
            (SAM, identity(2).verifying_key()),
        ];
        (
            Inbound::new(ROOM, &SPACE, members),
            EncryptionMonitor::new(),
            Encryption::default(),
        )
    }

    fn states(e: &Encryption) -> Vec<SenderState> {
        e.senders.iter().map(|s| s.state).collect()
    }

    #[test]
    fn alone_or_with_silent_members_nothing_is_verified_yet() {
        let mut out = Encryption::default();
        EncryptionMonitor::new().update(0, [], &mut out);
        assert_eq!(out.state, EncryptionState::Pending, "alone");

        let (inbound, mut monitor, mut out) = room();
        monitor.update(0, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Pending);
        assert_eq!(states(&out), [SenderState::Silent, SenderState::Silent]);
    }

    #[test]
    fn verified_once_everyone_heard_is_verified() {
        let (mut inbound, mut monitor, mut out) = room();
        let packets = loud_packets(&mut sender(1, ALEX));
        inbound.handle(&packets[0]);
        assert!(inbound.handle(&packets[1]).is_some());
        monitor.update(0, inbound.senders(), &mut out);
        // Sam is silent: that does not hold the room back.
        assert_eq!(states(&out), [SenderState::Verified, SenderState::Silent]);
        assert_eq!(out.state, EncryptionState::Verified);
        assert_eq!(out.senders[0].public_key, identity(1).verifying_key());
    }

    #[test]
    fn audio_before_its_announcement_is_pending() {
        let (mut inbound, mut monitor, mut out) = room();
        let alex = loud_packets(&mut sender(1, ALEX));
        inbound.handle(&alex[0]);
        inbound.handle(&alex[1]);
        // Sam's audio arrives, but their announcement was lost.
        let sam = loud_packets(&mut sender(2, SAM));
        assert_eq!(inbound.handle(&sam[1]), None);
        monitor.update(0, inbound.senders(), &mut out);
        assert_eq!(states(&out), [SenderState::Verified, SenderState::Waiting]);
        assert_eq!(out.state, EncryptionState::Pending);

        inbound.handle(&sam[0]);
        monitor.update(20_000, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Verified);
    }

    #[test]
    fn a_forged_announcement_fails_the_room_for_a_while() {
        let (mut inbound, mut monitor, mut out) = room();
        let alex = loud_packets(&mut sender(1, ALEX));
        inbound.handle(&alex[0]);
        monitor.update(0, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Verified);

        // Someone without alex's identity key announces a salt in alex's
        // name: the signature does not verify, and it is dropped.
        let forged = sender(9, ALEX).announce();
        assert_eq!(inbound.handle(&forged), None);
        monitor.update(100_000, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Failed);
        assert_eq!(out.senders[0].state, SenderState::Failed);
        assert_eq!(out.senders[0].bad_signature, 1);
        // Alex's own audio still plays: the forgery changed no key.
        assert!(inbound.handle(&alex[1]).is_some());

        monitor.update(100_000 + FAILURE_HOLD_US - 1, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Failed, "still shown");
        monitor.update(100_000 + FAILURE_HOLD_US, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Verified, "then cleared");
    }

    #[test]
    fn a_tampered_frame_fails_the_room_and_is_dropped() {
        let (mut inbound, mut monitor, mut out) = room();
        let mut alex = sender(1, ALEX);
        let packets = loud_packets(&mut alex);
        inbound.handle(&packets[0]);
        let mut tampered = packets[1].clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(inbound.handle(&tampered), None, "never played");
        monitor.update(0, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Failed);
        assert_eq!(out.senders[0].bad_tag, 1);

        // A new failure later starts the hold again.
        let mut again = loud_packets(&mut alex).pop().unwrap();
        again[20] ^= 1;
        inbound.handle(&again);
        monitor.update(FAILURE_HOLD_US, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Failed);
        assert_eq!(out.senders[0].bad_tag, 2);
        monitor.update(2 * FAILURE_HOLD_US, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Verified);
    }

    #[test]
    fn a_member_who_left_is_forgotten() {
        let (mut inbound, mut monitor, mut out) = room();
        inbound.handle(&loud_packets(&mut sender(1, ALEX))[0]);
        let forged = sender(9, SAM).announce();
        inbound.handle(&forged);
        monitor.update(0, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Failed);
        inbound.remove_peer(SAM);
        monitor.update(1, inbound.senders(), &mut out);
        assert_eq!(out.state, EncryptionState::Verified);
        assert_eq!(out.senders.len(), 1);
    }
}
