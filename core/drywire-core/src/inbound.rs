//! The receiving half of a client, up to the jitter buffer: packets from the
//! relay in, checked and decrypted frames out.
//!
//! [`Inbound`] keeps, for every other member of the room, the keys of its
//! last two key epochs (from its signed announcements) and one replay window
//! per epoch. For an audio packet it finds the key, decrypts, drops
//! duplicates, and splits the payload into frames (protocol sections 7
//! and 8). Everything it drops is counted under the codes of section 9.
//!
//! It does no I/O and keeps no clock. Its output feeds the jitter buffers of
//! the [`mixer`](crate::mixer).

use std::collections::HashMap;

use drywire_proto::crypto::{
    AudioKey, KEY_SIZE, ROOM_ID_SIZE, VerifyingKey, room_key, sender_key, verify_announce,
};
use drywire_proto::header::SALT_SIZE;
use drywire_proto::payload::AudioPayload;
use drywire_proto::replay::{ReplayWindow, Verdict};
use drywire_proto::{PacketType, parse};

use crate::jitter::Arrival;

/// What an accepted packet means for playback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A frame to buffer for this sender.
    Audio {
        /// The sender's session.
        session: u32,
        /// The frame and its timing fields.
        arrival: Arrival,
    },
    /// The sender announced a new salt for an epoch it had already
    /// announced: it restarted, so its old timeline is gone.
    Restarted {
        /// The sender's session.
        session: u32,
    },
}

/// Packets dropped by an [`Inbound`], by reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InboundStats {
    /// Audio packets accepted.
    pub audio: u64,
    /// Announcements accepted, repeats included.
    pub announces: u64,
    /// Header or size errors (`too_short` … `zero_session`).
    pub invalid: u64,
    /// Audio for a session and epoch with no key yet (protocol section 8.3).
    pub no_key: u64,
    /// Audio that failed to decrypt.
    pub bad_tag: u64,
    /// Audio whose payload breaks section 7.
    pub bad_framing: u64,
    /// Announcements that failed to verify, or from an unknown session.
    pub bad_signature: u64,
    /// Audio already seen.
    pub duplicate: u64,
    /// Audio older than the replay window.
    pub too_old: u64,
}

/// The key of one sender's epoch, and the packets seen in it.
struct EpochKey {
    epoch: u8,
    salt: [u8; SALT_SIZE],
    key: AudioKey,
    window: ReplayWindow,
}

/// What a receiver knows about one other member.
struct Peer {
    public_key: VerifyingKey,
    /// The last two epochs announced, newest last, so packets reordered
    /// across an epoch change still decrypt.
    epochs: Vec<EpochKey>,
}

/// Checks and decrypts the packets a client receives.
pub struct Inbound {
    room_id: [u8; ROOM_ID_SIZE],
    room_key: [u8; KEY_SIZE],
    peers: HashMap<u32, Peer>,
    stats: InboundStats,
}

impl Inbound {
    /// A receiver for one room. `members` are the other members' sessions
    /// and public keys; packets from anyone else are dropped.
    pub fn new(
        room_id: [u8; ROOM_ID_SIZE],
        space_key: &[u8; KEY_SIZE],
        members: impl IntoIterator<Item = (u32, VerifyingKey)>,
    ) -> Inbound {
        Inbound {
            room_id,
            room_key: room_key(space_key, &room_id),
            peers: members
                .into_iter()
                .map(|(session, public_key)| {
                    let epochs = Vec::with_capacity(2);
                    (session, Peer { public_key, epochs })
                })
                .collect(),
            stats: InboundStats::default(),
        }
    }

    /// The counters so far.
    pub fn stats(&self) -> InboundStats {
        self.stats
    }

    /// Handles an `audio` or `announce` packet from the relay. Other types
    /// are the relay link's business and are ignored here.
    pub fn handle(&mut self, packet: &[u8]) -> Option<Event> {
        let header = match parse(packet) {
            Ok(header) => header,
            Err(_) => {
                self.stats.invalid += 1;
                return None;
            }
        };
        match header.packet_type {
            PacketType::Announce => self.announce(header.session, header.key_epoch, packet),
            PacketType::Audio => self.audio(header.session, header.key_epoch, packet),
            _ => None,
        }
    }

    fn announce(&mut self, session: u32, epoch: u8, packet: &[u8]) -> Option<Event> {
        let Some(peer) = self.peers.get_mut(&session) else {
            self.stats.bad_signature += 1;
            return None;
        };
        let Ok(salt) = verify_announce(&peer.public_key, &self.room_id, packet) else {
            self.stats.bad_signature += 1;
            return None;
        };
        self.stats.announces += 1;
        let fresh = || EpochKey {
            epoch,
            salt,
            key: AudioKey::new(&sender_key(&self.room_key, &salt, &peer.public_key)),
            window: ReplayWindow::new(),
        };
        match peer.epochs.iter().position(|e| e.epoch == epoch) {
            // A repeat, sent so that a lost announcement does not matter.
            Some(i) if peer.epochs[i].salt == salt => None,
            // Same epoch, new salt: the sender restarted. Its older epochs
            // belong to the previous run.
            Some(_) => {
                let key = fresh();
                peer.epochs.clear();
                peer.epochs.push(key);
                Some(Event::Restarted { session })
            }
            None => {
                if peer.epochs.len() == 2 {
                    peer.epochs.remove(0);
                }
                let key = fresh();
                peer.epochs.push(key);
                None
            }
        }
    }

    fn audio(&mut self, session: u32, epoch: u8, packet: &[u8]) -> Option<Event> {
        let Some(epoch_key) = self
            .peers
            .get_mut(&session)
            .and_then(|peer| peer.epochs.iter_mut().find(|e| e.epoch == epoch))
        else {
            self.stats.no_key += 1;
            return None;
        };
        let plaintext = match epoch_key.key.open(packet) {
            Ok(plaintext) => plaintext,
            Err(_) => {
                self.stats.bad_tag += 1;
                return None;
            }
        };
        // After decryption, so forged packets cannot move the window.
        let header = parse(packet).expect("parsed before");
        match epoch_key.window.check(header.sequence) {
            Verdict::New => {}
            Verdict::Duplicate => {
                self.stats.duplicate += 1;
                return None;
            }
            Verdict::TooOld => {
                self.stats.too_old += 1;
                return None;
            }
        }
        let payload = match AudioPayload::decode(header.flags, &plaintext) {
            Ok(payload) => payload,
            Err(_) => {
                self.stats.bad_framing += 1;
                return None;
            }
        };
        self.stats.audio += 1;
        Some(Event::Audio {
            session,
            arrival: Arrival {
                timestamp: header.timestamp,
                end_of_spurt: header.is_end_of_spurt(),
                primary: payload.primary.to_vec(),
                redundant: payload.redundant.map(<[u8]>::to_vec),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opus::FrameSize;
    use crate::sender::{Sender, SenderConfig};
    use drywire_proto::crypto::SigningKey;

    const ROOM: [u8; 16] = [2; 16];
    const SPACE: [u8; 32] = [3; 32];

    fn sender() -> Sender {
        let config = SenderConfig::new(FrameSize::Ms5);
        Sender::new(SigningKey::from_bytes(&[1; 32]), 7, ROOM, &SPACE, config).unwrap()
    }

    fn inbound() -> Inbound {
        let public = SigningKey::from_bytes(&[1; 32]).verifying_key();
        Inbound::new(ROOM, &SPACE, [(7, public)])
    }

    fn loud_packets(s: &mut Sender) -> Vec<Vec<u8>> {
        let pcm: Vec<f32> = (0..240).map(|i| 0.3 * (i as f32 * 0.1).sin()).collect();
        let mut out = Vec::new();
        s.process(&pcm, &mut out).unwrap();
        out
    }

    fn is_audio(e: &Option<Event>) -> bool {
        matches!(e, Some(Event::Audio { session: 7, .. }))
    }

    #[test]
    fn audio_decrypts_once_its_announcement_arrived() {
        let mut s = sender();
        let mut r = inbound();
        let packets = loud_packets(&mut s); // announce, audio
        assert_eq!(r.handle(&packets[1]), None);
        assert_eq!(r.stats().no_key, 1);
        assert_eq!(r.handle(&packets[0]), None);
        assert!(is_audio(&r.handle(&packets[1])));
        // The same packet again is a duplicate.
        assert_eq!(r.handle(&packets[1]), None);
        assert_eq!(r.stats().duplicate, 1);
    }

    #[test]
    fn strangers_and_forgeries_are_dropped() {
        let mut s = sender();
        let packets = loud_packets(&mut s);
        let mut stranger = Inbound::new(ROOM, &SPACE, []);
        assert_eq!(stranger.handle(&packets[0]), None);
        assert_eq!(stranger.stats().bad_signature, 1);

        let mut r = inbound();
        r.handle(&packets[0]);
        let mut tampered = packets[1].clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(r.handle(&tampered), None);
        assert_eq!(r.stats().bad_tag, 1);
        assert_eq!(r.handle(&[1, 2, 3]), None);
        assert_eq!(r.stats().invalid, 1);
    }

    #[test]
    fn a_new_salt_for_a_known_epoch_means_a_restart() {
        let mut r = inbound();
        let mut first = sender();
        r.handle(&first.announce());
        assert_eq!(r.handle(&first.announce()), None, "a repeat is quiet");
        let mut restarted = sender();
        assert_eq!(
            r.handle(&restarted.announce()),
            Some(Event::Restarted { session: 7 })
        );
        // The old run's packets no longer decrypt; the new run's do.
        assert_eq!(r.handle(&loud_packets(&mut first)[1]), None);
        assert!(is_audio(&r.handle(&loud_packets(&mut restarted)[1])));
    }
}
