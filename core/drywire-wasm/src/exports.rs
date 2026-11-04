//! The functions the page calls, with plain numbers and one shared buffer.
//!
//! Without wasm-bindgen, a WebAssembly function only takes and returns
//! numbers. Bytes go through one buffer in the module's memory: the page
//! writes a function's input there, calls it, and reads its output from
//! the same place. [`dw_buffer`] gives its address. A negative result is
//! an error; [`dw_reason`] names the code a call returned.
//!
//! The module serves one page with one client, single-threaded, so the
//! buffer and the client live in thread-local cells, which on WebAssembly
//! without threads are plain globals, and no `unsafe` is needed.

use std::cell::RefCell;

use drywire_proto::header::{SALT_SIZE, SIGNATURE_SIZE};
use drywire_proto::invite::Invitation;
use drywire_proto::login::{CHALLENGE_SIZE, login_message};

use crate::{Client, Received, Setup};
use drywire_proto::crypto::room_key;

/// Size of the shared buffer: the largest datagram, 1200 bytes, fits.
pub const BUFFER_SIZE: usize = 2048;

/// Error: [`dw_start`] has not been called, or its input was invalid.
const NO_CLIENT: i32 = -1;
/// Error: a length larger than the buffer.
const TOO_LONG: i32 = -2;
/// Error: not the `hello_ack` the page waits for.
const NOT_THE_ACK: i32 = -3;
/// Error: not a valid invitation link.
const NOT_AN_INVITATION: i32 = -4;
/// A control packet from [`dw_receive`]: `CONTROL - packet type`.
const CONTROL: i32 = -10;
/// Errors from [`dw_receive`] start here: `-100 - index in REASONS`.
const DROPPED: i32 = -100;

/// The drop reasons [`Client::receive`] can give.
const REASONS: [&str; 14] = [
    "too_short",
    "too_large",
    "unsupported_version",
    "unknown_type",
    "bad_length",
    "zero_session",
    "bad_mac",
    "bad_tag",
    "bad_framing",
    "bad_signature",
    "unknown_sender",
    "no_key",
    "duplicate",
    "too_old",
];

thread_local! {
    static BUFFER: RefCell<[u8; BUFFER_SIZE]> = const { RefCell::new([0; BUFFER_SIZE]) };
    static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) };
}

fn with_buffer<T>(f: impl FnOnce(&mut [u8; BUFFER_SIZE]) -> T) -> T {
    BUFFER.with(|b| f(&mut b.borrow_mut()))
}

fn with_client(f: impl FnOnce(&mut Client, &mut [u8; BUFFER_SIZE]) -> i32) -> i32 {
    CLIENT.with(|c| match c.borrow_mut().as_mut() {
        Some(client) => with_buffer(|b| f(client, b)),
        None => NO_CLIENT,
    })
}

fn put(buffer: &mut [u8], bytes: &[u8]) -> i32 {
    buffer[..bytes.len()].copy_from_slice(bytes);
    bytes.len() as i32
}

fn array<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    b[at..at + N].try_into().expect("inside the buffer")
}

/// The address of the shared buffer, [`BUFFER_SIZE`] bytes long.
#[unsafe(no_mangle)]
pub extern "C" fn dw_buffer() -> *mut u8 {
    BUFFER.with(|b| b.as_ptr().cast::<u8>())
}

/// Input: an invitation link of `length` bytes (protocol section 14).
/// Writes `code (16) ‖ space key (32) ‖ instance`, the instance's
/// canonical origin in ASCII. Returns their length, or [`NOT_AN_INVITATION`].
/// Needs no [`dw_start`]: the page reads the link before it joins.
#[unsafe(no_mangle)]
pub extern "C" fn dw_invitation(length: u32) -> i32 {
    with_buffer(|b| {
        let Some(Ok(link)) = b.get(..length as usize).map(std::str::from_utf8) else {
            return NOT_AN_INVITATION;
        };
        let Ok(invitation) = Invitation::parse(link) else {
            return NOT_AN_INVITATION;
        };
        let n = put(b, &invitation.code) as usize;
        let n = n + put(&mut b[n..], &invitation.space_key) as usize;
        n as i32 + put(&mut b[n..], invitation.instance.as_bytes())
    })
}

/// Input: `challenge (32) ‖ audience`, the audience being the instance's
/// canonical origin, `audience_length` bytes. Writes the message the
/// identity key signs to log in (protocol section 13.2). Returns its
/// length. Needs no [`dw_start`].
#[unsafe(no_mangle)]
pub extern "C" fn dw_login_message(audience_length: u32) -> i32 {
    with_buffer(|b| {
        let end = CHALLENGE_SIZE + audience_length as usize;
        let Some(Ok(audience)) = b.get(CHALLENGE_SIZE..end).map(std::str::from_utf8) else {
            return TOO_LONG;
        };
        let message = login_message(&array(b, 0), audience);
        if message.len() > BUFFER_SIZE {
            return TOO_LONG;
        }
        put(b, &message)
    })
}

/// Starts the client of a session in a space with a space key. Input:
/// `secret (32) ‖ space key (32) ‖ room id (16) ‖ public key (32) ‖ salt
/// (32)`. Returns 0, or [`NO_CLIENT`] if the public key is invalid.
#[unsafe(no_mangle)]
pub extern "C" fn dw_start(session: u32) -> i32 {
    start(session, true)
}

/// [`dw_start`] with the room's key itself in place of the space key: a
/// space whose keys come from its MLS group (ADR 0052).
#[unsafe(no_mangle)]
pub extern "C" fn dw_start_room(session: u32) -> i32 {
    start(session, false)
}

fn start(session: u32, from_space_key: bool) -> i32 {
    let setup = with_buffer(|b| {
        let key: [u8; 32] = array(b, 32);
        let room_id = array(b, 64);
        Setup {
            session,
            secret: array(b, 0),
            room_key: if from_space_key {
                room_key(&key, &room_id)
            } else {
                key
            },
            room_id,
            public_key: array(b, 80),
            salt: array::<SALT_SIZE>(b, 112),
        }
    });
    let client = Client::new(&setup);
    let ok = client.is_some();
    CLIENT.with(|c| *c.borrow_mut() = client);
    if ok { 0 } else { NO_CLIENT }
}

/// Writes the `hello` with this counter. Returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn dw_hello(counter: u64) -> i32 {
    with_client(|c, b| put(b, &c.hello(counter)))
}

/// Writes a `ping`. The page reads the `pong` itself: its client time is
/// the 8 bytes at offset 16. Returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn dw_ping(number: u32, client_time: u64) -> i32 {
    with_client(|c, b| put(b, &c.ping(number, client_time)))
}

/// Whether the packet of `length` bytes in the buffer is the `hello_ack`
/// to the hello with this counter: 0 if it is, [`NOT_THE_ACK`] if not.
#[unsafe(no_mangle)]
pub extern "C" fn dw_is_hello_ack(length: u32, counter: u64) -> i32 {
    with_client(|c, b| match b.get(..length as usize) {
        Some(packet) if c.is_hello_ack(packet, counter) => 0,
        Some(_) => NOT_THE_ACK,
        None => TOO_LONG,
    })
}

/// Writes the message the identity key signs for the announcement.
/// Returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn dw_announcement_message() -> i32 {
    with_client(|c, b| put(b, &c.announcement_message()))
}

/// Input: the 64-byte signature of the announcement message. Writes the
/// `announce` packet. Returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn dw_announce() -> i32 {
    with_client(|c, b| {
        let signature = array::<SIGNATURE_SIZE>(b, 0);
        put(b, &c.announce(&signature))
    })
}

/// Input: one Opus frame of `length` bytes. Writes the sealed audio
/// packet. Returns its length.
#[unsafe(no_mangle)]
pub extern "C" fn dw_seal(length: u32, timestamp: u32, end_of_spurt: u32) -> i32 {
    with_client(|c, b| match b.get(..length as usize) {
        Some(frame)
            if !frame.is_empty() && frame.len() <= drywire_proto::payload::MAX_FRAME_SIZE =>
        {
            let packet = c.seal(frame, timestamp, end_of_spurt != 0);
            put(b, &packet)
        }
        _ => TOO_LONG,
    })
}

/// Input: a member's 32-byte public key. Starts accepting their packets.
/// Returns 0, or [`NO_CLIENT`] for an invalid key.
#[unsafe(no_mangle)]
pub extern "C" fn dw_add_peer(session: u32) -> i32 {
    with_client(|c, b| {
        if c.add_peer(session, &array(b, 0)) {
            0
        } else {
            NO_CLIENT
        }
    })
}

/// Forgets a member who left.
#[unsafe(no_mangle)]
pub extern "C" fn dw_remove_peer(session: u32) -> i32 {
    with_client(|c, _| {
        c.remove_peer(session);
        0
    })
}

/// Input: one packet of `length` bytes from the relay. For audio, writes
/// its 16-byte header then its Opus frame and returns their total length.
/// Returns 0 for an accepted announcement, `-10 - packet type` for a
/// control packet the page reads itself (`-13` for `hello_ack`, `-16` for
/// `pong`), and `-100 - reason` for a dropped packet.
#[unsafe(no_mangle)]
pub extern "C" fn dw_receive(length: u32) -> i32 {
    with_client(|c, b| {
        let Some(packet) = b.get(..length as usize) else {
            return TOO_LONG;
        };
        let packet = packet.to_vec();
        match c.receive(&packet) {
            Received::Audio { header, frame } => {
                let n = put(b, &header.to_bytes());
                n + put(&mut b[n as usize..], frame)
            }
            Received::Announced { .. } => 0,
            Received::Control(t) => CONTROL - t as i32,
            Received::Dropped(reason) => {
                DROPPED - REASONS.iter().position(|r| *r == reason).unwrap_or(0) as i32
            }
        }
    })
}

/// Writes the name of an error code a call returned, in ASCII. Returns
/// its length.
#[unsafe(no_mangle)]
pub extern "C" fn dw_reason(code: i32) -> i32 {
    let name = match code {
        NO_CLIENT => "no_client",
        TOO_LONG => "too_long",
        NOT_THE_ACK => "not_the_ack",
        NOT_AN_INVITATION => "not_an_invitation",
        c if c <= DROPPED => REASONS
            .get((DROPPED - c) as usize)
            .copied()
            .unwrap_or("unknown"),
        _ => "unknown",
    };
    with_buffer(|b| put(b, name.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drop_reason_has_a_code() {
        // Each reason the client can give maps to its own code and back.
        for (i, reason) in REASONS.iter().enumerate() {
            let n = dw_reason(DROPPED - i as i32) as usize;
            assert_eq!(with_buffer(|b| b[..n].to_vec()), reason.as_bytes());
        }
    }

    #[test]
    fn invitations_and_login_messages_need_no_client() {
        let invitation = Invitation {
            instance: "https://voice.example.org".into(),
            code: [1; 16],
            space_key: [2; 32],
        };
        let link = invitation.to_link();
        with_buffer(|b| put(b, link.as_bytes()));
        let n = dw_invitation(link.len() as u32) as usize;
        let out = with_buffer(|b| b[..n].to_vec());
        assert_eq!(&out[..16], &[1; 16]);
        assert_eq!(&out[16..48], &[2; 32]);
        assert_eq!(&out[48..], b"https://voice.example.org");

        with_buffer(|b| put(b, b"not a link"));
        assert_eq!(dw_invitation(10), NOT_AN_INVITATION);

        let audience = b"https://voice.example.org";
        with_buffer(|b| {
            b[..32].fill(7);
            b[32..32 + audience.len()].copy_from_slice(audience)
        });
        let n = dw_login_message(audience.len() as u32) as usize;
        let want = login_message(&[7; 32], "https://voice.example.org");
        assert_eq!(with_buffer(|b| b[..n].to_vec()), want);
    }

    #[test]
    fn calls_before_start_fail() {
        assert_eq!(dw_hello(1), NO_CLIENT);
    }
}
