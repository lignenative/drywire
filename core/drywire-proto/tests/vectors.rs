//! Decodes every shared test vector the way a receiver would and compares the
//! result with `vectors/manifest.json` (protocol section 12).
//!
//! This mirrors `TestVectorsDecode` in `server/internal/vectors`: the Go and
//! the Rust implementations share no code, so agreeing on these files is what
//! proves they speak the same protocol.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use drywire_proto::control::{self, SECRET_SIZE};
use drywire_proto::crypto::{self, AudioKey, KEY_SIZE, ROOM_ID_SIZE};
use drywire_proto::header::{SALT_SIZE, VERSION};
use drywire_proto::payload::AudioPayload;
use drywire_proto::replay::ReplayWindow;
use drywire_proto::{Error, PacketType, parse};
use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Manifest {
    protocol_version: u8,
    inputs: Inputs,
    epochs: Vec<Epoch>,
    vectors: Vec<Value>,
    replay_window: Vec<ReplayStep>,
}

#[derive(Deserialize)]
struct Inputs {
    session: u32,
    session_secret: String,
    space_key: String,
    room_id: String,
    sender_seed: String,
    sender_public_key: String,
    room_key: String,
}

#[derive(Deserialize)]
struct Epoch {
    epoch: u8,
    sender_salt: String,
    sender_key: String,
    signature: String,
}

#[derive(Deserialize)]
struct ReplayStep {
    sequence: u32,
    verdict: String,
}

/// What a receiver needs to decode the vectors: the keys from the manifest,
/// each checked against its derivation before use.
struct Keys {
    session_secret: [u8; SECRET_SIZE],
    room_id: [u8; ROOM_ID_SIZE],
    sender_public_key: VerifyingKey,
    audio_keys: HashMap<u8, AudioKey>,
}

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors")
}

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd-length hex: {s}");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn hex_array<const N: usize>(s: &str) -> [u8; N] {
    hex(s).try_into().expect("hex of the right length")
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn load() -> Manifest {
    let raw = std::fs::read_to_string(vectors_dir().join("manifest.json"))
        .expect("manifest.json is readable (run make vectors)");
    serde_json::from_str(&raw).expect("manifest.json parses")
}

/// Checks every key derivation of the manifest and returns the keys.
fn keys(m: &Manifest) -> Keys {
    let inputs = &m.inputs;
    let identity = SigningKey::from_bytes(&hex_array(&inputs.sender_seed));
    let public = identity.verifying_key();
    assert_eq!(
        to_hex(public.as_bytes()),
        inputs.sender_public_key,
        "sender_public_key does not match sender_seed"
    );

    let room_id: [u8; ROOM_ID_SIZE] = hex_array(&inputs.room_id);
    let room_key = crypto::room_key(&hex_array(&inputs.space_key), &room_id);
    assert_eq!(
        to_hex(&room_key),
        inputs.room_key,
        "room_key does not match HKDF(space_key, room_id)"
    );

    let mut audio_keys = HashMap::new();
    for e in &m.epochs {
        let salt: [u8; SALT_SIZE] = hex_array(&e.sender_salt);
        let key: [u8; KEY_SIZE] = crypto::sender_key(&room_key, &salt, &public);
        assert_eq!(to_hex(&key), e.sender_key, "epoch {}: sender_key", e.epoch);

        let message = crypto::announcement_message(&room_id, inputs.session, e.epoch, &salt);
        let signature = Signature::from_bytes(&hex_array(&e.signature));
        assert!(
            public.verify(&message, &signature).is_ok(),
            "epoch {}: announcement signature does not verify",
            e.epoch
        );
        // Ed25519 is deterministic: signing again gives the same bytes.
        let packet = crypto::new_announce(&identity, &room_id, inputs.session, e.epoch, &salt);
        assert_eq!(control::announce_fields(&packet).1, signature.to_bytes());

        audio_keys.insert(e.epoch, AudioKey::new(&key));
    }

    Keys {
        session_secret: hex_array(&inputs.session_secret),
        room_id,
        sender_public_key: public,
        audio_keys,
    }
}

/// Turns a packet into the manifest fields a receiver can read from it,
/// stopping at the first error. `fields` is filled as far as decoding got,
/// exactly like the Go test does.
fn decode(
    packet: &[u8],
    keys: &Keys,
    fields: &mut serde_json::Map<String, Value>,
) -> Result<(), Error> {
    let h = parse(packet)?;
    fields.insert(
        "header".into(),
        json!({
            "version": h.version,
            "type": h.packet_type.name(),
            "flags": h.flags,
            "key_epoch": h.key_epoch,
            "session": h.session,
            "sequence": h.sequence,
            "timestamp": h.timestamp,
        }),
    );
    match h.packet_type {
        PacketType::Hello | PacketType::HelloAck => {
            let counter = control::verify_handshake(packet, &keys.session_secret)?;
            fields.insert("hello".into(), json!({ "counter": counter }));
        }
        PacketType::Ping => {
            fields.insert(
                "ping".into(),
                json!({ "client_time": control::ping_client_time(packet) }),
            );
        }
        PacketType::Pong => {
            let p = control::decode_pong(packet);
            fields.insert(
                "pong".into(),
                json!({
                    "client_time": p.client_time,
                    "audio_received": p.audio_received,
                    "audio_lost": p.audio_lost,
                }),
            );
        }
        PacketType::Audio => {
            let key = &keys.audio_keys[&h.key_epoch];
            let plaintext = key.open(packet)?;
            let payload = AudioPayload::decode(h.flags, &plaintext)?;
            let mut audio = json!({ "plaintext": to_hex(&plaintext) });
            if let Some(redundant) = payload.redundant {
                audio["redundant_frame"] = to_hex(redundant).into();
            }
            audio["primary_frame"] = to_hex(payload.primary).into();
            fields.insert("audio".into(), audio);
        }
        PacketType::Announce => {
            let salt = crypto::verify_announce(&keys.sender_public_key, &keys.room_id, packet)?;
            let (_, signature) = control::announce_fields(packet);
            fields.insert(
                "announce".into(),
                json!({ "sender_salt": to_hex(&salt), "signature": to_hex(&signature) }),
            );
        }
        PacketType::Keepalive => {}
    }
    Ok(())
}

#[test]
fn vectors_decode() {
    let m = load();
    assert_eq!(
        m.protocol_version, VERSION,
        "vectors are for another protocol version"
    );
    let keys = keys(&m);

    for want in &m.vectors {
        let name = want["name"].as_str().expect("name");
        let file = want["file"].as_str().expect("file");
        let packet = std::fs::read(vectors_dir().join(file)).expect("vector file is readable");

        // Start from the descriptive fields, then add what decoding finds.
        let mut got = serde_json::Map::new();
        for k in ["name", "file", "description"] {
            got.insert(k.into(), want[k].clone());
        }
        match (decode(&packet, &keys, &mut got), want.get("error")) {
            (Ok(()), None) => {}
            (Err(e), Some(code)) if code == e.code() => {
                got.insert("error".into(), code.clone());
            }
            (result, expected) => {
                panic!("{name}: decoding gave {result:?}, manifest expects {expected:?}")
            }
        }
        assert_eq!(
            &Value::Object(got),
            want,
            "{name}: decoded fields differ from the manifest"
        );
    }
}

#[test]
fn replay_window_follows_the_manifest() {
    let m = load();
    let mut w = ReplayWindow::new();
    for (i, step) in m.replay_window.iter().enumerate() {
        let got = w.check(step.sequence).name();
        assert_eq!(got, step.verdict, "step {i}: sequence {}", step.sequence);
    }
}

/// Builds every valid packet again from the manifest's fields and checks the
/// bytes are identical. Decoding alone would not catch an encoder that
/// writes something the decoder happens to accept.
#[test]
fn encoders_reproduce_the_valid_vectors() {
    let m = load();
    let keys = keys(&m);
    let identity = SigningKey::from_bytes(&hex_array(&m.inputs.sender_seed));

    for v in m.vectors.iter().filter(|v| v.get("error").is_none()) {
        let name = v["name"].as_str().unwrap();
        let want = std::fs::read(vectors_dir().join(v["file"].as_str().unwrap())).unwrap();
        let h = parse(&want).unwrap();
        let got: Vec<u8> = match h.packet_type {
            PacketType::Hello => control::new_hello(
                h.session,
                v["hello"]["counter"].as_u64().unwrap(),
                &keys.session_secret,
            )
            .to_vec(),
            PacketType::HelloAck => control::new_hello_ack(
                h.session,
                v["hello"]["counter"].as_u64().unwrap(),
                &keys.session_secret,
            )
            .to_vec(),
            PacketType::Keepalive => control::new_keepalive(h.session).to_vec(),
            PacketType::Ping => control::new_ping(
                h.session,
                h.sequence,
                v["ping"]["client_time"].as_u64().unwrap(),
            )
            .to_vec(),
            PacketType::Pong => control::new_pong(h.session, control::decode_pong(&want)).to_vec(),
            PacketType::Announce => {
                let salt = hex_array(v["announce"]["sender_salt"].as_str().unwrap());
                crypto::new_announce(&identity, &keys.room_id, h.session, h.key_epoch, &salt)
                    .to_vec()
            }
            PacketType::Audio => {
                let audio = &v["audio"];
                let primary = hex(audio["primary_frame"].as_str().unwrap());
                let redundant = audio
                    .get("redundant_frame")
                    .map(|r| hex(r.as_str().unwrap()));
                let payload = AudioPayload {
                    redundant: redundant.as_deref(),
                    primary: &primary,
                };
                let (plaintext, _) = payload.encode();
                assert_eq!(to_hex(&plaintext), audio["plaintext"], "{name}: plaintext");
                keys.audio_keys[&h.key_epoch].seal(&h, &plaintext)
            }
        };
        assert_eq!(
            to_hex(&got),
            to_hex(&want),
            "{name}: encoder output differs"
        );
    }
}
