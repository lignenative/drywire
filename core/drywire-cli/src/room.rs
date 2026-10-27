//! `drywire-cli new-room`: everything a group needs to talk through its own
//! relay in phase 1, before the control plane exists.
//!
//! It writes the relay's rooms file (ADR 0004) and one client configuration
//! per member (ADR 0006), all with fresh random keys. The files hold
//! secrets: give each member only their own file, over a private channel.
//!
//! With the hidden `--dev` flag it writes the public development files
//! instead: fixed, well-known keys that match `server/rooms.dev.json`,
//! marked `dev_only` so that neither side accepts them beyond this machine.

use std::path::Path;

use drywire_core::config::{ClientConfig, Member, to_hex};
use drywire_proto::crypto::SigningKey;
use serde::Serialize;

use crate::args::NewRoom;

#[derive(Serialize)]
struct RoomsFile {
    dev_only: bool,
    rooms: Vec<Room>,
}

#[derive(Serialize)]
struct Room {
    name: String,
    members: Vec<RoomMember>,
}

#[derive(Serialize)]
struct RoomMember {
    session: u32,
    secret: String,
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    bytes
}

/// One member's secrets: random, or the fixed development ones. The
/// development session secret is the one `server/rooms.dev.json` lists:
/// 32 bytes of `0x11 × session`.
fn secrets(dev: bool, index: usize, session: u32) -> ([u8; 32], [u8; 32]) {
    if dev {
        ([session as u8 * 0x11; 32], [0xa1 + index as u8; 32])
    } else {
        (random(), random())
    }
}

pub fn run(args: NewRoom) -> Result<(), String> {
    let (room_id, space_key) = if args.dev {
        ([0x0f; 16], [0x5a; 32])
    } else {
        (random(), random())
    };
    let sessions = (0..args.members.len()).map(|i| args.first_session + i as u32);
    let people: Vec<(String, u32, [u8; 32], SigningKey)> = args
        .members
        .iter()
        .zip(sessions)
        .enumerate()
        .map(|(i, (name, session))| {
            let (secret, seed) = secrets(args.dev, i, session);
            (name.clone(), session, secret, SigningKey::from_bytes(&seed))
        })
        .collect();
    let members: Vec<Member> = people
        .iter()
        .map(|(name, session, _, identity)| Member {
            session: *session,
            name: name.clone(),
            public_key: identity.verifying_key(),
        })
        .collect();

    std::fs::create_dir_all(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))?;
    let rooms = RoomsFile {
        dev_only: args.dev,
        rooms: vec![Room {
            name: args.room.clone(),
            members: people
                .iter()
                .map(|(_, session, secret, _)| RoomMember {
                    session: *session,
                    secret: to_hex(secret),
                })
                .collect(),
        }],
    };
    let json = serde_json::to_string_pretty(&rooms).expect("serializable") + "\n";
    write_secret(&args.out.join("rooms.json"), &json)?;

    for (name, session, secret, identity) in &people {
        let config = ClientConfig {
            dev_only: args.dev,
            relay: args.relay.clone(),
            room_id,
            space_key,
            session: *session,
            session_secret: *secret,
            identity: identity.clone(),
            members: members.clone(),
        };
        write_secret(&args.out.join(format!("{name}.json")), &config.to_json())?;
    }
    println!(
        "Wrote rooms.json and one configuration per member to {}.",
        args.out.display()
    );
    if !args.dev {
        println!(
            "rooms.json goes to the relay (DRYWIRE_ROOMS); each member gets only their own file. \
             They hold secrets: send them privately."
        );
    }
    Ok(())
}

/// Writes a file readable by its owner only, where the system has owners.
fn write_secret(path: &Path, contents: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut file, contents.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}
