//! A space's group through its life, as the apps will drive it: the
//! delivery service is played by passing the bytes along by hand.

use drywire_mls::{Device, Error, Group, PendingDevice};
use ed25519_dalek::SigningKey;

const SPACE: [u8; 16] = [5; 16];
const ROOM: [u8; 16] = [6; 16];

/// A device, made from the key it logs in with (ADR 0055).
fn device(n: u8) -> Device {
    Device::new(&SigningKey::from_bytes(&[n; 32])).unwrap()
}

fn pending(device: &Device) -> PendingDevice {
    PendingDevice {
        device: device.public_key(),
        key_package: device.key_package().unwrap(),
    }
}

fn anyone(_: &[u8; 32]) -> bool {
    true
}

#[test]
fn a_space_group_lives_its_life() {
    // Alex's PC and phone, and Sam's PC.
    let pc = device(1);
    let phone = device(2);
    let sams = device(3);
    assert_eq!(
        pc.public_key(),
        SigningKey::from_bytes(&[1; 32]).verifying_key().to_bytes()
    );

    let mut g_pc = Group::create(&pc, SPACE).unwrap();
    assert_eq!((g_pc.epoch(), g_pc.space()), (0, SPACE));

    // The phone and Sam wait; the PC adds both in one commit.
    let commit = g_pc
        .add(&pc, &[pending(&phone), pending(&sams)], anyone)
        .unwrap();
    assert_eq!(commit.welcomed, vec![phone.public_key(), sams.public_key()]);
    g_pc.confirm(&pc).unwrap();
    let welcome = commit.welcome.unwrap();
    let g_phone = Group::join(&phone, &welcome, anyone).unwrap();
    let mut g_sam = Group::join(&sams, &welcome, anyone).unwrap();
    assert_eq!(g_pc.epoch(), 1);
    assert_eq!(g_sam.leaves().len(), 3);
    let k1 = g_pc.room_key(&pc, &ROOM).unwrap();
    assert_eq!(k1, g_phone.room_key(&phone, &ROOM).unwrap());
    assert_eq!(k1, g_sam.room_key(&sams, &ROOM).unwrap());
    assert_ne!(
        k1,
        g_pc.room_key(&pc, &[7; 16]).unwrap(),
        "each room its key"
    );
    drop(g_phone);

    // The phone's state survives a restart.
    let phone = Device::load(&phone.save()).unwrap();
    let mut g_phone = Group::load(&phone, SPACE).unwrap().unwrap();
    assert_eq!(g_phone.room_key(&phone, &ROOM).unwrap(), k1);

    // Sam renews their leaf: a new epoch, a new key for all.
    let renewal = g_sam.renew(&sams).unwrap();
    g_sam.confirm(&sams).unwrap();
    g_pc.apply(&pc, &renewal.commit, anyone).unwrap();
    g_phone.apply(&phone, &renewal.commit, anyone).unwrap();
    let k2 = g_pc.room_key(&pc, &ROOM).unwrap();
    assert_ne!(k1, k2);
    assert_eq!(k2, g_sam.room_key(&sams, &ROOM).unwrap());
    assert_eq!(k2, g_phone.room_key(&phone, &ROOM).unwrap());

    // Sam leaves the space: the members' devices no longer include theirs,
    // and the phone prunes it. The new key is out of Sam's reach.
    let sam_key = sams.public_key();
    let still = |d: &[u8; 32]| *d != sam_key;
    let removal = g_phone.prune(&phone, still).unwrap().unwrap();
    g_phone.confirm(&phone).unwrap();
    g_pc.apply(&pc, &removal.commit, still).unwrap();
    let k3 = g_pc.room_key(&pc, &ROOM).unwrap();
    assert_eq!(k3, g_phone.room_key(&phone, &ROOM).unwrap());
    assert_ne!(k3, k2);
    assert_eq!(g_pc.leaves().len(), 2);
    assert!(
        g_pc.prune(&pc, still).unwrap().is_none(),
        "nothing left to prune"
    );
    assert_eq!(
        g_sam.room_key(&sams, &ROOM).unwrap(),
        k2,
        "Sam stays in the epoch before"
    );
}

#[test]
fn the_second_commit_of_an_epoch_waits_its_turn() {
    let pc = device(1);
    let sams = device(2);
    let late = device(3);
    let mut g_pc = Group::create(&pc, SPACE).unwrap();
    let commit = g_pc.add(&pc, &[pending(&sams)], anyone).unwrap();
    g_pc.confirm(&pc).unwrap();
    let mut g_sam = Group::join(&sams, &commit.welcome.unwrap(), anyone).unwrap();

    // Both commit in epoch 1; the service takes the PC's.
    let first = g_pc.renew(&pc).unwrap();
    let lost = g_sam.add(&sams, &[pending(&late)], anyone).unwrap();
    g_pc.confirm(&pc).unwrap();
    // Sam's is refused (409): drop it, catch up, and commit again.
    g_sam.abandon(&sams).unwrap();
    g_sam.apply(&sams, &first.commit, anyone).unwrap();
    assert!(
        g_pc.apply(&pc, &lost.commit, anyone).is_err(),
        "a commit for an old epoch"
    );
    let again = g_sam.add(&sams, &[pending(&late)], anyone).unwrap();
    g_sam.confirm(&sams).unwrap();
    g_pc.apply(&pc, &again.commit, anyone).unwrap();
    let g_late = Group::join(&late, &again.welcome.unwrap(), anyone).unwrap();
    assert_eq!(g_late.epoch(), 3);
    assert_eq!(
        g_late.room_key(&late, &ROOM).unwrap(),
        g_pc.room_key(&pc, &ROOM).unwrap()
    );
}

#[test]
fn a_key_package_must_be_a_members_device_s() {
    let pc = device(1);
    let sams = device(2);
    let other = device(3);
    let mut g = Group::create(&pc, SPACE).unwrap();

    // Posted under another device's name.
    let mut p = pending(&sams);
    p.device = other.public_key();
    assert_eq!(g.add(&pc, &[p], anyone).err(), Some(Error::WrongDevice));
    // A device the control plane does not list for any member.
    let sam_key = sams.public_key();
    assert_eq!(
        g.add(&pc, &[pending(&sams)], |d| *d != sam_key).err(),
        Some(Error::NotMember)
    );
    // Not a key package.
    let mut p = pending(&sams);
    p.key_package = vec![1, 2, 3];
    assert_eq!(g.add(&pc, &[p], anyone).err(), Some(Error::Malformed));
    assert_eq!(g.epoch(), 0);
}

#[test]
fn a_device_joins_only_a_group_of_members() {
    let pc = device(1);
    let sams = device(2);
    let mut g = Group::create(&pc, SPACE).unwrap();
    let commit = g.add(&pc, &[pending(&sams)], anyone).unwrap();
    // The control plane does not list Alex's PC: a leaf for a stranger.
    let alex = pc.public_key();
    let only_sam = |d: &[u8; 32]| *d != alex;
    assert_eq!(
        Group::join(&sams, commit.welcome.as_ref().unwrap(), only_sam).err(),
        Some(Error::NotMember)
    );
}

#[test]
fn a_saved_state_is_checked() {
    let pc = device(1);
    let saved = pc.save();
    assert!(Device::load(&saved[..saved.len() - 1]).is_err());
    let mut newer = saved.clone();
    newer[0] = 2;
    assert!(Device::load(&newer).is_err());
    assert_eq!(Device::load(&saved).unwrap().public_key(), pc.public_key());
}

#[test]
fn a_welcome_for_a_full_space_fits_the_delivery_service() {
    // 50 members with two devices each, the size the server's 64 KiB
    // limit on one message must hold (protocol section 16).
    let pc = device(1);
    let mut g = Group::create(&pc, SPACE).unwrap();
    let devices: Vec<Device> = (2..101u8).map(device).collect();
    let all: Vec<PendingDevice> = devices[..98].iter().map(pending).collect();
    let commit = g.add(&pc, &all, anyone).unwrap();
    g.confirm(&pc).unwrap();
    // The 100th device comes alone: its welcome holds the whole tree.
    let last = g.add(&pc, &[pending(&devices[98])], anyone).unwrap();
    let welcome = last.welcome.unwrap().len();
    eprintln!(
        "first commit {} B, one more device: commit {} B, welcome {welcome} B",
        commit.commit.len(),
        last.commit.len()
    );
    assert!(welcome <= 64 * 1024, "welcome of {welcome} B");
}
