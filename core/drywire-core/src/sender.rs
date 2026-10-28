//! The sending half of a client: microphone frames in, packets out.
//!
//! For every captured frame the [`Sender`] asks the voice gate whether to
//! send it, encodes it with Opus, wraps it in an audio payload (with the
//! previous frame when redundancy is on), and seals it with the sender key
//! of the current epoch. It also decides when to repeat the signed
//! announcement that lets receivers derive that key (protocol section 8.3,
//! ADR 0007).
//!
//! It does no I/O and reads no clock: the frame count is its clock, which is
//! what the timestamp measures anyway. The caller sends the packets it
//! returns, in order.
//!
//! It runs in the capture callback, so after [`Sender::new`] it never
//! allocates: frames, plaintexts and packets are written into buffers made
//! once, with room for the largest of each (ADR 0017).

use drywire_proto::Header;
use drywire_proto::PacketType;
use drywire_proto::crypto::{
    AudioKey, KEY_SIZE, ROOM_ID_SIZE, SigningKey, new_announce, room_key, sender_key,
};
use drywire_proto::header::{ANNOUNCE_SIZE, FLAG_END_OF_SPURT, MAX_PACKET_SIZE, SALT_SIZE};
use drywire_proto::payload::AudioPayload;

use crate::gate::{GateConfig, GateDecision, VoiceGate};
use crate::opus::{Encoder, FrameSize, MAX_FRAME_BYTES, OpusError, SAMPLE_RATE};

/// A sender starts a new key epoch before its sequence reaches this value
/// (protocol section 8.3).
pub const EPOCH_SEQUENCE_LIMIT: u32 = 0x8000_0000;

/// How a [`Sender`] encodes and sends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SenderConfig {
    /// Opus frame size.
    pub frame: FrameSize,
    /// Opus bitrate in bits per second.
    pub bitrate: u32,
    /// Voice gate thresholds and hold.
    pub gate: GateConfig,
    /// Whether each packet also carries the previous frame (spec section 7).
    pub redundancy: bool,
}

impl SenderConfig {
    /// The defaults for `frame`-sized frames: 64 kb/s, the default gate, no
    /// redundancy.
    pub fn new(frame: FrameSize) -> SenderConfig {
        SenderConfig {
            frame,
            bitrate: 64_000,
            gate: GateConfig::default(),
            redundancy: false,
        }
    }
}

/// Counters kept by a [`Sender`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SenderStats {
    /// Frames captured, sent or not.
    pub frames: u64,
    /// Audio packets produced.
    pub audio: u64,
    /// Announcements produced.
    pub announces: u64,
    /// Talk spurts started.
    pub spurts: u64,
    /// Key epochs started, the first one included.
    pub epochs: u64,
}

/// The packets one frame produced, in the order to send them: at most an
/// announcement and an audio packet. Its buffers are made once, with room
/// for the largest packet, and reused for every frame.
pub struct Packets {
    buffers: [Vec<u8>; 2],
    len: usize,
}

impl Packets {
    /// Room for the packets of one frame.
    pub fn new() -> Packets {
        Packets {
            buffers: [
                Vec::with_capacity(MAX_PACKET_SIZE),
                Vec::with_capacity(MAX_PACKET_SIZE),
            ],
            len: 0,
        }
    }

    /// How many packets the last frame produced.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the last frame produced no packet.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The packets, in order.
    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        self.buffers[..self.len].iter().map(Vec::as_slice)
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    /// The buffer of the next packet, emptied.
    fn push(&mut self) -> &mut Vec<u8> {
        let buffer = &mut self.buffers[self.len];
        self.len += 1;
        buffer.clear();
        buffer
    }
}

impl Default for Packets {
    fn default() -> Packets {
        Packets::new()
    }
}

/// The sending state of one client in one room.
pub struct Sender {
    session: u32,
    identity: SigningKey,
    room_id: [u8; ROOM_ID_SIZE],
    room_key: [u8; KEY_SIZE],
    config: SenderConfig,
    gate: VoiceGate,
    encoder: Encoder,
    epoch: u8,
    salt: [u8; SALT_SIZE],
    key: AudioKey,
    /// Sequence of the next audio packet in this epoch.
    sequence: u32,
    /// Timestamp of the next frame, sent or not.
    timestamp: u32,
    /// Timestamp of the last announcement, while the gate is open.
    announced_at: Option<u32>,
    /// The frame being encoded.
    frame: Vec<u8>,
    /// The previous packet's primary frame, valid while `has_previous`: it
    /// serves as the redundant copy while in the same spurt and epoch.
    previous: Vec<u8>,
    has_previous: bool,
    /// The payload being sealed.
    plaintext: Vec<u8>,
    stats: SenderStats,
}

impl Sender {
    /// A sender for `session`, at the start of epoch 0 with a fresh salt.
    /// Send [`Sender::announce`] once the session is bound, before audio.
    pub fn new(
        identity: SigningKey,
        session: u32,
        room_id: [u8; ROOM_ID_SIZE],
        space_key: &[u8; KEY_SIZE],
        config: SenderConfig,
    ) -> Result<Sender, OpusError> {
        let room_key = room_key(space_key, &room_id);
        let salt = random_salt();
        let key = AudioKey::new(&sender_key(&room_key, &salt, &identity.verifying_key()));
        Ok(Sender {
            session,
            identity,
            room_id,
            room_key,
            gate: VoiceGate::new(config.gate, config.frame),
            encoder: Encoder::new(config.frame, config.bitrate)?,
            config,
            epoch: 0,
            salt,
            key,
            sequence: 0,
            // Any start value is valid; a random one keeps restarts apart.
            timestamp: u32::from_be_bytes(random_salt()[..4].try_into().expect("4 bytes")),
            announced_at: None,
            frame: Vec::with_capacity(MAX_FRAME_BYTES),
            previous: Vec::with_capacity(MAX_FRAME_BYTES),
            has_previous: false,
            plaintext: Vec::with_capacity(2 + 2 * MAX_FRAME_BYTES),
            stats: SenderStats {
                epochs: 1,
                ..SenderStats::default()
            },
        })
    }

    /// The configuration this sender was made with.
    pub fn config(&self) -> SenderConfig {
        self.config
    }

    /// The counters so far.
    pub fn stats(&self) -> SenderStats {
        self.stats
    }

    /// Whether the voice gate is open.
    pub fn is_talking(&self) -> bool {
        self.gate.is_open()
    }

    /// Changes the voice gate's thresholds and hold, from the next frame
    /// on (see [`VoiceGate::set_config`]). It does not allocate.
    pub fn set_gate(&mut self, gate: GateConfig) {
        self.gate.set_config(gate);
        self.config.gate = self.gate.config();
    }

    /// The signed announcement of the current epoch.
    pub fn announce(&mut self) -> Vec<u8> {
        self.signed_announce().to_vec()
    }

    fn signed_announce(&mut self) -> [u8; ANNOUNCE_SIZE] {
        self.stats.announces += 1;
        new_announce(
            &self.identity,
            &self.room_id,
            self.session,
            self.epoch,
            &self.salt,
        )
    }

    /// Processes one captured frame of exactly the configured size, and
    /// puts the packets to send, in order, in `out`, replacing what it held.
    pub fn process(&mut self, pcm: &[f32], out: &mut Packets) -> Result<(), OpusError> {
        out.clear();
        let timestamp = self.timestamp;
        self.timestamp = timestamp.wrapping_add(self.config.frame.samples() as u32);
        self.stats.frames += 1;

        let GateDecision::Send {
            start_of_spurt,
            end_of_spurt,
        } = self.gate.process(pcm)
        else {
            return Ok(());
        };
        // Encode first: a frame Opus refuses changes nothing below.
        self.encoder.encode_into(pcm, &mut self.frame)?;

        if start_of_spurt {
            self.stats.spurts += 1;
            self.has_previous = false; // never a copy on a spurt's first packet
        }
        // Before the first packet of every spurt and of every epoch, then
        // once per second.
        let mut announce = start_of_spurt
            || self
                .announced_at
                .is_none_or(|at| timestamp.wrapping_sub(at) >= SAMPLE_RATE);
        if self.sequence >= EPOCH_SEQUENCE_LIMIT {
            self.start_epoch();
            announce = true;
        }
        if announce {
            let signed = self.signed_announce();
            out.push().extend_from_slice(&signed);
            self.announced_at = Some(timestamp);
        }

        let header = Header {
            flags: if end_of_spurt { FLAG_END_OF_SPURT } else { 0 },
            key_epoch: self.epoch,
            sequence: self.sequence,
            timestamp,
            ..Header::new(PacketType::Audio, self.session)
        };
        let payload = AudioPayload {
            redundant: (self.config.redundancy && self.has_previous)
                .then_some(self.previous.as_slice()),
            primary: &self.frame,
        };
        let redundant_flag = payload.encode_into(&mut self.plaintext);
        let header = Header {
            flags: header.flags | redundant_flag,
            ..header
        };
        self.key.seal_into(&header, &self.plaintext, out.push());
        self.sequence += 1;
        self.stats.audio += 1;

        if end_of_spurt {
            self.has_previous = false;
            self.announced_at = None;
        } else {
            // This frame is the next packet's copy; its buffer takes the
            // next frame.
            std::mem::swap(&mut self.previous, &mut self.frame);
            self.has_previous = true;
        }
        Ok(())
    }

    /// Draws a new salt and moves to the next epoch, whose sequence starts
    /// at 0 (protocol section 8.3).
    fn start_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.salt = random_salt();
        self.key = AudioKey::new(&sender_key(
            &self.room_key,
            &self.salt,
            &self.identity.verifying_key(),
        ));
        self.sequence = 0;
        // The redundant copy must come from the same epoch.
        self.has_previous = false;
        self.stats.epochs += 1;
    }
}

/// 32 bytes from the operating system's random number generator.
fn random_salt() -> [u8; SALT_SIZE] {
    let mut salt = [0u8; SALT_SIZE];
    getrandom::fill(&mut salt).expect("the operating system provides random bytes");
    salt
}

#[cfg(test)]
mod tests {
    use super::*;
    use drywire_proto::crypto::verify_announce;
    use drywire_proto::parse;

    const FRAME: FrameSize = FrameSize::Ms5;

    fn sender(redundancy: bool) -> Sender {
        let config = SenderConfig {
            redundancy,
            ..SenderConfig::new(FRAME)
        };
        Sender::new(
            SigningKey::from_bytes(&[1; 32]),
            7,
            [2; 16],
            &[3; 32],
            config,
        )
        .unwrap()
    }

    fn loud() -> Vec<f32> {
        (0..FRAME.samples())
            .map(|i| 0.3 * (i as f32 * 0.1).sin())
            .collect()
    }

    /// Types of the packets one frame produced.
    fn types(s: &mut Sender, pcm: &[f32]) -> Vec<(PacketType, u8)> {
        let mut out = Packets::new();
        s.process(pcm, &mut out).unwrap();
        out.iter()
            .map(|p| {
                let h = parse(p).unwrap();
                (h.packet_type, h.flags)
            })
            .collect()
    }

    #[test]
    fn silence_sends_nothing_but_advances_the_timestamp() {
        let mut s = sender(false);
        let start = s.timestamp;
        assert!(types(&mut s, &[0.0; 240]).is_empty());
        assert_eq!(s.timestamp, start.wrapping_add(240));
        assert_eq!(s.sequence, 0);
    }

    #[test]
    fn a_spurt_starts_with_an_announcement_and_ends_with_the_flag() {
        let mut s = sender(false);
        let a = PacketType::Announce;
        let audio = PacketType::Audio;
        assert_eq!(types(&mut s, &loud()), [(a, 0), (audio, 0)]);
        assert_eq!(types(&mut s, &loud()), [(audio, 0)]);
        // The default hold is 150 ms: 29 quiet frames still in the spurt,
        // the 30th is its last.
        for _ in 0..29 {
            assert_eq!(types(&mut s, &[0.0; 240]), [(audio, 0)]);
        }
        assert_eq!(types(&mut s, &[0.0; 240]), [(audio, FLAG_END_OF_SPURT)]);
        assert!(types(&mut s, &[0.0; 240]).is_empty());
        // The next spurt announces again.
        assert_eq!(types(&mut s, &loud()), [(a, 0), (audio, 0)]);
        assert_eq!(s.stats().spurts, 2);
    }

    #[test]
    fn a_long_spurt_repeats_the_announcement_every_second() {
        let mut s = sender(false);
        let mut announces = 0;
        let mut out = Packets::new();
        for _ in 0..(3 * 200) {
            s.process(&loud(), &mut out).unwrap();
            announces += out
                .iter()
                .filter(|p| parse(p).unwrap().packet_type == PacketType::Announce)
                .count();
        }
        // At 0 s, 1 s and 2 s of a 3-second spurt.
        assert_eq!(announces, 3);
    }

    #[test]
    fn redundancy_copies_the_previous_frame_of_the_same_spurt() {
        let mut s = sender(true);
        let mut out = Packets::new();
        s.process(&loud(), &mut out).unwrap();
        let first = parse(out.iter().last().unwrap()).unwrap();
        assert!(!first.is_redundant(), "no copy on the first packet");
        s.process(&loud(), &mut out).unwrap();
        let second: Vec<_> = out.iter().collect();
        assert_eq!(second.len(), 1);
        assert!(parse(second[0]).unwrap().is_redundant());
    }

    #[test]
    fn the_announcement_verifies_and_a_new_epoch_restarts_the_sequence() {
        let mut s = sender(false);
        let public = SigningKey::from_bytes(&[1; 32]).verifying_key();
        let salt = verify_announce(&public, &[2; 16], &s.announce()).unwrap();
        assert_eq!(salt, s.salt);

        s.sequence = EPOCH_SEQUENCE_LIMIT;
        let mut out = Packets::new();
        s.process(&loud(), &mut out).unwrap();
        let headers: Vec<_> = out.iter().map(|p| parse(p).unwrap()).collect();
        // The new epoch is announced before its first audio packet.
        assert_eq!(headers[0].packet_type, PacketType::Announce);
        assert_eq!(headers[0].key_epoch, 1);
        let audio = headers.last().unwrap();
        assert_eq!((audio.key_epoch, audio.sequence), (1, 0));
        let first = out.iter().next().unwrap();
        assert_ne!(verify_announce(&public, &[2; 16], first).unwrap(), salt);
    }
}
