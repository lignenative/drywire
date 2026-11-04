//! Accounts by e-mail and a space's MLS group, against the real server
//! (ADRs 0052 and 0055): sign in on two devices, choose a user name,
//! invite by user name, and let a new member's device into the group.

#[path = "../../testsupport/relay.rs"]
mod relay;

use std::thread::sleep;
use std::time::Duration;

use drywire_control::{Client, Error, Event, PendingKeyPackage};
use drywire_mls::{Device, Group, PendingDevice};
use drywire_proto::crypto::SigningKey;
use relay::Relay;

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

/// Signs a device in by e-mail, as the apps do; returns whether the
/// account is new. Each sign-in spends a few logins' worth of the
/// server's rate budget, so it takes a breath first.
fn sign_in(server: &Relay, client: &Client, email: &str) -> bool {
    sleep(Duration::from_millis(800));
    client.request_email_code(email, "en").unwrap();
    let code = server.mail_code(email);
    client.verify_email_code(email, &code).unwrap().new_account
}

#[test]
fn friends_sign_up_by_mail_and_meet_in_a_group() {
    let server = Relay::start();
    let instance = server.instance();
    let pc = Client::new(&instance, key(1)).unwrap();
    let phone = Client::new(&instance, key(2)).unwrap();
    let sam = Client::new(&instance, key(3)).unwrap();

    // Alex signs up on the PC, with a wrong code first.
    pc.request_email_code("alex@example.com", "en").unwrap();
    match pc.verify_email_code("alex@example.com", "000000") {
        Err(Error::WrongCode { tries_left: 4 }) => {}
        other => panic!("a wrong code: {other:?}"),
    }
    let code = server.mail_code("alex@example.com");
    assert!(
        pc.verify_email_code("alex@example.com", &code)
            .unwrap()
            .new_account
    );
    let me = pc.set_username("alex", "Alex").unwrap();
    assert_eq!(me.username.as_deref(), Some("alex"));
    assert_eq!(me.email.as_deref(), Some("alex@example.com"));

    // The phone signs in to the same account.
    assert!(!sign_in(&server, &phone, "alex@example.com"));
    assert_eq!(phone.devices().unwrap().len(), 2);

    // Sam signs up, and cannot take "alex".
    assert!(sign_in(&server, &sam, "sam@example.com"));
    match sam.set_username("Alex", "") {
        Err(Error::UsernameTaken { suggestions }) => assert_eq!(suggestions.len(), 2),
        other => panic!("a taken name: {other:?}"),
    }
    sam.set_username("sam", "Sam").unwrap();

    // Alex creates a space and its group, and invites Sam by user name.
    let team = pc.create_space("Team", "general", "Alex").unwrap();
    let space = team.space.id;
    let alex_device = Device::new(&key(1)).unwrap();
    let mut group = Group::create(&alex_device, space.0).unwrap();
    assert_eq!(pc.find_user("SAM").unwrap().unwrap().name, "Sam");
    pc.invite_user(&space, "sam").unwrap();
    let waiting = sam.invitations().unwrap();
    assert_eq!(waiting[0].space_name, "Team");
    let joined = sam.accept_invitation(&space).unwrap();
    assert_eq!(joined.members.len(), 2);

    // Sam's device asks to join the group; Alex's app hears of it.
    let mut events = pc.events(&space).unwrap();
    assert!(matches!(
        events.next().unwrap().unwrap(),
        Event::Snapshot(_)
    ));
    let sam_device = Device::new(&key(3)).unwrap();
    sam.post_key_package(
        &space,
        &sam.public_key(),
        &sam_device.key_package().unwrap(),
    )
    .unwrap();
    assert_eq!(events.next().unwrap().unwrap(), Event::Mls(0));

    // Alex adds every waiting device the member list vouches for.
    let members = pc.space(&space).unwrap().members;
    let is_member = |d: &[u8; 32]| members.iter().any(|m| m.devices.iter().any(|k| k.0 == *d));
    let pending: Vec<PendingDevice> = pc
        .key_packages(&space)
        .unwrap()
        .into_iter()
        .map(
            |PendingKeyPackage {
                 device,
                 key_package,
             }| PendingDevice {
                device,
                key_package,
            },
        )
        .collect();
    assert_eq!(pending.len(), 1);
    let commit = group.add(&alex_device, &pending, is_member).unwrap();
    let epoch = pc
        .post_commit(
            &space,
            group.epoch(),
            &commit.commit,
            commit.welcome.as_deref(),
            &commit.welcomed,
        )
        .unwrap();
    group.confirm(&alex_device).unwrap();
    assert_eq!((epoch, group.epoch()), (1, 1));

    // The same commit again is too late.
    match pc.post_commit(&space, 0, &commit.commit, None, &[]) {
        Err(Error::StaleEpoch { current: 1 }) => {}
        other => panic!("a stale commit: {other:?}"),
    }

    // Sam's device joins from its welcome: both hold the same room key.
    let (at, welcome) = sam.welcome(&space, &sam.public_key()).unwrap().unwrap();
    assert_eq!(at, 1);
    let sams_group = Group::join(&sam_device, &welcome, is_member).unwrap();
    let room = team.rooms[0].id.0;
    assert_eq!(
        sams_group.room_key(&sam_device, &room).unwrap(),
        group.room_key(&alex_device, &room).unwrap()
    );
    assert_eq!(pc.commits(&space, 0).unwrap().len(), 1);
}

#[test]
fn the_group_keeps_itself_in_step() {
    use drywire_control::{GroupState, SpaceGroup};
    let server = Relay::start();
    let instance = server.instance();
    let alex = Client::new(&instance, key(1)).unwrap();
    let sam = Client::new(&instance, key(2)).unwrap();
    sign_in(&server, &alex, "alex@example.com");
    alex.set_username("alex", "").unwrap();
    sign_in(&server, &sam, "sam@example.com");
    sam.set_username("sam", "").unwrap();
    let team = alex.create_space("Team", "general", "alex").unwrap();
    let (space, room) = (team.space.id, team.rooms[0].id);
    alex.invite_user(&space, "sam").unwrap();
    sam.accept_invitation(&space).unwrap();

    let (alex_device, sam_device) = (Device::new(&key(1)).unwrap(), Device::new(&key(2)).unwrap());
    let mut alex_group = SpaceGroup::new(space, &alex_device).unwrap();
    let mut sam_group = SpaceGroup::new(space, &sam_device).unwrap();

    // The owner's device makes the group; Sam's asks to join, and is let
    // in at the owner's next round.
    assert_eq!(
        alex_group.sync(&alex, &alex_device, true).unwrap(),
        GroupState::Ready(1)
    );
    assert_eq!(
        sam_group.sync(&sam, &sam_device, false).unwrap(),
        GroupState::Waiting
    );
    assert_eq!(
        alex_group.sync(&alex, &alex_device, true).unwrap(),
        GroupState::Ready(2)
    );
    assert_eq!(
        sam_group.sync(&sam, &sam_device, false).unwrap(),
        GroupState::Ready(2)
    );
    let key2 = alex_group.room_key(&alex_device, &room).unwrap();
    assert_eq!(sam_group.room_key(&sam_device, &room), Some(key2));

    // A device that restarts finds its group in its saved state.
    let saved = Device::load(&sam_device.save()).unwrap();
    assert_eq!(SpaceGroup::new(space, &saved).unwrap().epoch(), Some(2));

    // Sam leaves the space: Alex's next round takes their device out, and
    // the room key changes.
    sam.leave_space(&space).unwrap();
    assert_eq!(
        alex_group.sync(&alex, &alex_device, true).unwrap(),
        GroupState::Ready(3)
    );
    assert_ne!(alex_group.room_key(&alex_device, &room), Some(key2));
}
