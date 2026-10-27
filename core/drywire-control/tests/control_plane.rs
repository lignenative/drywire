//! The client against the real server: a space, an invitation link, two
//! members in a room, presence events, and the end of an identity.

#[path = "../../testsupport/relay.rs"]
mod relay;

use drywire_control::{Client, Event, Hex};
use drywire_proto::crypto::SigningKey;
use drywire_proto::invite::Invitation;
use relay::Relay;

#[test]
fn two_members_meet_through_an_invitation_link() {
    let server = Relay::start();
    let alex = Client::new(&server.instance(), SigningKey::from_bytes(&[1; 32])).unwrap();
    let sam = Client::new(&server.instance(), SigningKey::from_bytes(&[2; 32])).unwrap();

    // Alex creates a space; the space key never leaves the clients.
    let space_key = [0x5a; 32];
    let team = alex.create_space("Team", "general", "alex").unwrap();
    assert_eq!(team.role, "owner");
    let room = team.rooms[0].clone();
    let (invitation, info) = alex
        .invite(&team.space.id, &space_key, None, Some(1))
        .unwrap();
    assert_eq!(info.max_uses, 1);
    let link = invitation.to_link();

    // Sam reads the link, redeems its code, and has the key.
    let read = Invitation::parse(&link).unwrap();
    assert_eq!(read.instance, server.instance());
    assert_eq!(read.space_key, space_key);
    let joined = sam.redeem(&read.code, "sam").unwrap();
    assert_eq!(joined.space.id, team.space.id);
    assert_eq!(joined.members.len(), 2);

    // Alex follows presence while both join the room.
    let mut events = alex.events(&team.space.id).unwrap();
    assert_eq!(events.next().unwrap().unwrap(), Event::Snapshot(vec![]));
    let a = alex.join(&room.id).unwrap();
    let b = sam.join(&room.id).unwrap();
    assert_ne!(a.session, b.session);
    assert_eq!(b.sessions.len(), 2);
    assert_eq!(b.relay, server.udp.to_string());
    match events.next().unwrap().unwrap() {
        Event::Join(p) => assert_eq!((p.session, p.name.as_str()), (a.session, "alex")),
        other => panic!("{other:?}"),
    }
    match events.next().unwrap().unwrap() {
        Event::Join(p) => {
            assert_eq!(p.session, b.session);
            assert_eq!(p.public_key, sam.public_key());
        }
        other => panic!("{other:?}"),
    }

    sam.leave().unwrap();
    assert!(matches!(events.next().unwrap().unwrap(), Event::Leave(p) if p.session == b.session));

    // The rights of spec section 9.
    assert_eq!(sam.rename("Sam B").unwrap().name, "Sam B");
    let export = sam.export().unwrap();
    assert_eq!(export["identity"]["name"], "Sam B");
    sam.delete_identity().unwrap();
    assert!(sam.me().unwrap().is_none(), "a deleted identity is gone");
    assert_eq!(alex.space(&team.space.id).unwrap().members.len(), 1);
}

#[test]
fn errors_carry_the_servers_message() {
    let server = Relay::start();
    let kim = Client::new(&server.instance(), SigningKey::from_bytes(&[3; 32])).unwrap();
    let err = kim.redeem(&[0; 16], "kim").unwrap_err();
    assert_eq!(err.status(), Some(404));
    assert!(
        err.to_string().contains("unknown, expired or used up"),
        "{err}"
    );
    let err = kim.space(&Hex([0; 16])).unwrap_err();
    assert_eq!(err.status(), Some(404));
}

#[test]
fn plain_http_is_for_this_machine_only() {
    let key = SigningKey::from_bytes(&[4; 32]);
    assert!(Client::new("http://127.0.0.1:10050", key.clone()).is_ok());
    assert!(Client::new("http://[::1]:10050", key.clone()).is_ok());
    assert!(Client::new("http://localhost", key.clone()).is_ok());
    assert!(Client::new("http://voice.example.org", key.clone()).is_err());
    assert!(Client::new("https://voice.example.org", key).is_ok());
}
