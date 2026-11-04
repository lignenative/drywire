//! The Drywire protocol for the web client, compiled to WebAssembly
//! (ADR 0040).
//!
//! The browser has no UDP and no libopus, but it can run this: the packets,
//! the end-to-end encryption, the replay window, the invitation links and
//! the login message of `drywire-proto`, the
//! very code the native clients run and the shared test vectors check. The
//! page does the rest itself: WebTransport for the datagrams, WebCodecs for
//! Opus, WebCrypto for the identity key, which never enters this module.
//!
//! [`Client`] is plain Rust, tested natively. [`exports`] is the thin layer
//! of C functions the page calls; it needs neither wasm-bindgen nor any
//! JavaScript glue, so the module has no dependency beyond `drywire-proto`.
//!
//! The receiving half repeats, in a few lines, what `drywire-core`'s
//! `Inbound` does. `drywire-core` does not build for the browser yet: it
//! binds libopus and the operating systems' audio APIs. Once its pure
//! parts do, this module uses them instead.

pub mod exports;

use std::collections::HashMap;

use drywire_proto::control::{SECRET_SIZE, new_hello, new_ping, verify_handshake};
use drywire_proto::crypto::{
    ANNOUNCEMENT_MESSAGE_SIZE, AudioKey, KEY_SIZE, ROOM_ID_SIZE, VerifyingKey,
    announcement_message, sender_key, verify_announce,
};
use drywire_proto::header::{
    ANNOUNCE_SIZE, FLAG_END_OF_SPURT, HELLO_SIZE, PING_SIZE, SALT_SIZE, SIGNATURE_SIZE,
};
use drywire_proto::payload::AudioPayload;
use drywire_proto::replay::{ReplayWindow, Verdict};
use drywire_proto::{Header, PacketType, parse};

/// What a client starts from: what the control plane and the invitation
/// gave it, and the salt of its first key epoch.
pub struct Setup {
    /// The session the control plane handed out.
    pub session: u32,
    /// Its session secret.
    pub secret: [u8; SECRET_SIZE],
    /// The room's key: from the space's MLS group (ADR 0052), or derived
    /// from the space key of an older space (`room_key`).
    pub room_key: [u8; KEY_SIZE],
    /// The room's id.
    pub room_id: [u8; ROOM_ID_SIZE],
    /// This client's Ed25519 public key. The private key stays in
    /// WebCrypto; the page signs the announcement itself.
    pub public_key: [u8; 32],
    /// 32 random bytes for key epoch 0, from `crypto.getRandomValues`.
    pub salt: [u8; SALT_SIZE],
}

/// What [`Client::receive`] made of a packet.
#[derive(Debug, PartialEq, Eq)]
pub enum Received<'a> {
    /// A decrypted Opus frame from another member.
    Audio {
        /// Its header: session, sequence, timestamp, flags.
        header: Header,
        /// The primary frame. A redundant copy, if any, is not used yet.
        frame: &'a [u8],
    },
    /// An accepted sender announcement: its sender's audio now decrypts.
    Announced {
        /// The sender's session.
        session: u32,
    },
    /// A packet for the relay link (`hello_ack`, `pong`), which the page
    /// reads itself.
    Control(PacketType),
    /// Dropped, for the reason named (the codes of protocol section 9 and
    /// of the replay window).
    Dropped(&'static str),
}

/// The keys of one sender's key epoch, and the packets seen in it.
struct EpochKey {
    epoch: u8,
    salt: [u8; SALT_SIZE],
    key: AudioKey,
    window: ReplayWindow,
}

struct Peer {
    public_key: VerifyingKey,
    /// The last two epochs announced, newest last, so that packets
    /// reordered across an epoch change still decrypt.
    epochs: Vec<EpochKey>,
}

/// One member of a room, as the web client is: it seals its own audio and
/// opens everyone else's.
pub struct Client {
    session: u32,
    secret: [u8; SECRET_SIZE],
    room_id: [u8; ROOM_ID_SIZE],
    room_key: [u8; KEY_SIZE],
    salt: [u8; SALT_SIZE],
    key: AudioKey,
    sequence: u32,
    peers: HashMap<u32, Peer>,
    /// The plaintext of the last audio packet received; frames borrow it.
    plaintext: Vec<u8>,
}

impl Client {
    /// A client in key epoch 0. Fails if the public key is not a valid
    /// Ed25519 point.
    pub fn new(setup: &Setup) -> Option<Client> {
        let public_key = VerifyingKey::from_bytes(&setup.public_key).ok()?;
        let room_key = setup.room_key;
        let key = AudioKey::new(&sender_key(&room_key, &setup.salt, &public_key));
        Some(Client {
            session: setup.session,
            secret: setup.secret,
            room_id: setup.room_id,
            room_key,
            salt: setup.salt,
            key,
            sequence: 0,
            peers: HashMap::new(),
            plaintext: Vec::new(),
        })
    }

    /// The `hello` that binds the session (protocol section 6.1).
    pub fn hello(&self, counter: u64) -> [u8; HELLO_SIZE] {
        new_hello(self.session, counter, &self.secret)
    }

    /// A `ping` to measure the round trip to the relay (section 6.4).
    /// `client_time` comes back unchanged in the `pong`.
    pub fn ping(&self, number: u32, client_time: u64) -> [u8; PING_SIZE] {
        new_ping(self.session, number, client_time)
    }

    /// Whether `packet` is the relay's `hello_ack` to the hello with this
    /// counter (section 6.2).
    pub fn is_hello_ack(&self, packet: &[u8], counter: u64) -> bool {
        matches!(parse(packet), Ok(h) if h.packet_type == PacketType::HelloAck && h.session == self.session)
            && verify_handshake(packet, &self.secret) == Ok(counter)
    }

    /// The message this client's identity key signs to announce epoch 0
    /// (section 8.3). The page signs it with WebCrypto once per epoch and
    /// sends the same announcement every time: any valid signature of the
    /// message will do, so there is no need to sign again.
    pub fn announcement_message(&self) -> [u8; ANNOUNCEMENT_MESSAGE_SIZE] {
        announcement_message(&self.room_id, self.session, 0, &self.salt)
    }

    /// The `announce` packet, from the signature of
    /// [`Client::announcement_message`] (section 6.7).
    pub fn announce(&self, signature: &[u8; SIGNATURE_SIZE]) -> [u8; ANNOUNCE_SIZE] {
        drywire_proto::control::new_announce(self.session, 0, &self.salt, signature)
    }

    /// Seals one Opus frame into the next audio packet (sections 6.6 and
    /// 8.4). `timestamp` is the frame's first sample, at 48 kHz.
    pub fn seal(&mut self, frame: &[u8], timestamp: u32, end_of_spurt: bool) -> Vec<u8> {
        let header = Header {
            flags: if end_of_spurt { FLAG_END_OF_SPURT } else { 0 },
            sequence: self.sequence,
            timestamp,
            ..Header::new(PacketType::Audio, self.session)
        };
        self.sequence += 1;
        let (payload, _) = AudioPayload {
            redundant: None,
            primary: frame,
        }
        .encode();
        self.key.seal(&header, &payload)
    }

    /// Starts accepting a member's packets, with the public key the
    /// control plane gave for their session.
    pub fn add_peer(&mut self, session: u32, public_key: &[u8; 32]) -> bool {
        let Ok(public_key) = VerifyingKey::from_bytes(public_key) else {
            return false;
        };
        let epochs = Vec::with_capacity(2);
        self.peers.insert(session, Peer { public_key, epochs });
        true
    }

    /// Forgets a member who left, and their keys.
    pub fn remove_peer(&mut self, session: u32) {
        self.peers.remove(&session);
    }

    /// Checks one packet from the relay and, for audio, decrypts it.
    pub fn receive(&mut self, packet: &[u8]) -> Received<'_> {
        let header = match parse(packet) {
            Ok(header) => header,
            Err(e) => return Received::Dropped(e.code()),
        };
        match header.packet_type {
            PacketType::Announce => self.announced(header, packet),
            PacketType::Audio => self.audio(header, packet),
            other => Received::Control(other),
        }
    }

    fn announced(&mut self, header: Header, packet: &[u8]) -> Received<'_> {
        let Some(peer) = self.peers.get_mut(&header.session) else {
            return Received::Dropped("unknown_sender");
        };
        let Ok(salt) = verify_announce(&peer.public_key, &self.room_id, packet) else {
            return Received::Dropped("bad_signature");
        };
        let epoch = header.key_epoch;
        let known = peer.epochs.iter().position(|e| e.epoch == epoch);
        if known.is_some_and(|i| peer.epochs[i].salt == salt) {
            return Received::Announced {
                session: header.session,
            };
        }
        if known.is_some() {
            // Same epoch, new salt: the sender restarted, and its older
            // epochs belong to the previous run.
            peer.epochs.clear();
        } else if peer.epochs.len() == 2 {
            peer.epochs.remove(0);
        }
        peer.epochs.push(EpochKey {
            epoch,
            salt,
            key: AudioKey::new(&sender_key(&self.room_key, &salt, &peer.public_key)),
            window: ReplayWindow::new(),
        });
        Received::Announced {
            session: header.session,
        }
    }

    fn audio(&mut self, header: Header, packet: &[u8]) -> Received<'_> {
        let Some(epoch) = self
            .peers
            .get_mut(&header.session)
            .and_then(|p| p.epochs.iter_mut().find(|e| e.epoch == header.key_epoch))
        else {
            return Received::Dropped("no_key");
        };
        let Ok(plaintext) = epoch.key.open(packet) else {
            return Received::Dropped("bad_tag");
        };
        // After decryption, so that forged packets cannot move the window.
        match epoch.window.check(header.sequence) {
            Verdict::New => {}
            other => return Received::Dropped(other.name()),
        }
        self.plaintext = plaintext;
        match AudioPayload::decode(header.flags, &self.plaintext) {
            Ok(payload) => Received::Audio {
                header,
                frame: payload.primary,
            },
            Err(e) => Received::Dropped(e.code()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drywire_proto::crypto::{SigningKey, new_announce};
    use drywire_proto::header::HEADER_SIZE;

    /// The signature WebCrypto would give, taken from the announcement
    /// the native code signs for the same epoch.
    fn sign(identity: &SigningKey, setup: &Setup) -> [u8; SIGNATURE_SIZE] {
        let native = new_announce(identity, &setup.room_id, setup.session, 0, &setup.salt);
        native[HEADER_SIZE + SALT_SIZE..].try_into().unwrap()
    }

    fn setup(session: u32, seed: u8) -> (Setup, SigningKey) {
        let identity = SigningKey::from_bytes(&[seed; 32]);
        let setup = Setup {
            session,
            secret: [seed ^ 0x55; 32],
            room_key: drywire_proto::crypto::room_key(&[7; 32], &[1; 16]),
            room_id: [9; 16],
            public_key: identity.verifying_key().to_bytes(),
            salt: [seed ^ 0xaa; 32],
        };
        (setup, identity)
    }

    #[test]
    fn two_clients_hear_each_other() {
        let (alex_setup, alex_id) = setup(1, 1);
        let (sam_setup, _) = setup(2, 2);
        let mut alex = Client::new(&alex_setup).unwrap();
        let mut sam = Client::new(&sam_setup).unwrap();
        assert!(sam.add_peer(1, &alex_setup.public_key));

        // The page signs the message it is given; the packet built from
        // that signature is the one the native client sends.
        let announce = alex.announce(&sign(&alex_id, &alex_setup));
        assert_eq!(
            announce,
            new_announce(&alex_id, &[9; 16], 1, 0, &alex_setup.salt)
        );
        let public = VerifyingKey::from_bytes(&alex_setup.public_key).unwrap();
        let message = alex.announcement_message();
        assert_eq!(
            verify_announce(&public, &[9; 16], &announce),
            Ok(alex_setup.salt),
            "the message signed is {message:x?}"
        );

        let audio = alex.seal(b"opus frame", 480, false);
        assert_eq!(sam.receive(&audio), Received::Dropped("no_key"));
        assert_eq!(sam.receive(&announce), Received::Announced { session: 1 });
        match sam.receive(&audio) {
            Received::Audio { header, frame } => {
                assert_eq!(
                    (header.session, header.sequence, header.timestamp),
                    (1, 0, 480)
                );
                assert_eq!(frame, b"opus frame");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(sam.receive(&audio), Received::Dropped("duplicate"));
        let next = alex.seal(b"next", 960, true);
        assert!(
            matches!(sam.receive(&next), Received::Audio { header, .. } if header.sequence == 1 && header.is_end_of_spurt())
        );
    }

    #[test]
    fn strangers_and_forgeries_are_dropped() {
        let (alex_setup, alex_id) = setup(1, 1);
        let (sam_setup, _) = setup(2, 2);
        let alex = Client::new(&alex_setup).unwrap();
        let mut sam = Client::new(&sam_setup).unwrap();
        let announce = alex.announce(&sign(&alex_id, &alex_setup));

        assert_eq!(sam.receive(&announce), Received::Dropped("unknown_sender"));
        sam.add_peer(1, &sam_setup.public_key); // the wrong key for session 1
        assert_eq!(sam.receive(&announce), Received::Dropped("bad_signature"));
        assert_eq!(sam.receive(&[1, 2, 3]), Received::Dropped("too_short"));
    }

    #[test]
    fn the_hello_ack_must_answer_the_last_hello() {
        let (s, _) = setup(5, 5);
        let client = Client::new(&s).unwrap();
        let ack = drywire_proto::control::new_hello_ack(5, 42, &s.secret);
        assert!(client.is_hello_ack(&ack, 42));
        assert!(!client.is_hello_ack(&ack, 41));
        assert!(
            !client.is_hello_ack(&client.hello(42), 42),
            "a hello is not an ack"
        );
    }
}
