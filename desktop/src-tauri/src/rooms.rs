//! Who sits in each room of the spaces this device knows, for the side
//! menu, which lists names under every room, including the ones this
//! device is not in.
//!
//! The control plane already says it: a space's event stream begins with
//! a snapshot of everyone in its rooms, then reports each arrival and
//! departure (protocol section 13.6). [`Rooms`] follows the stream of
//! every known space on a thread of its own and keeps the result in
//! memory, never on disk. Each change calls a function the app gives,
//! which pushes the spaces to the window: no polling, no network request
//! to show a name.
//!
//! The server holds at most 8 streams per identity, and one more ends the
//! oldest (ADR 0047). A call opens one for its own space, and a
//! reconnection briefly two, so at most [`MAX_PER_INSTANCE`] spaces of an
//! instance are followed: never so many that the app's own new stream
//! would end its call's. The others show their rooms without names, as
//! before.
//!
//! A space's call follows its own stream (`session.rs`): the two never
//! share one, so nothing here can delay or confuse the call.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use drywire_control::{Client, Event, Id, Presence};

use crate::session::printable;

/// Called after every change, on a stream's thread.
pub type Changed = Arc<dyn Fn() + Send + Sync>;

/// The most spaces followed on one instance: two streams below the
/// server's limit of 8, for a call's own and a reconnection's.
pub const MAX_PER_INSTANCE: usize = 6;

/// The first pause before opening a stream again, and the longest: a
/// server that is down is not asked every two seconds for long.
const RETRY_FIRST: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// A space, by its instance and id.
type SpaceKey = (String, Id);

/// One followed space.
struct Followed {
    /// Everyone in its rooms, by session; `None` until the first snapshot,
    /// and while its stream cannot be opened.
    sessions: Option<HashMap<u32, Presence>>,
}

/// Every followed space. Dropping it stops the streams' threads, each at
/// its stream's next event or keepalive.
pub struct Rooms {
    spaces: Arc<Mutex<HashMap<SpaceKey, Followed>>>,
    changed: Changed,
    /// The flag the running threads watch; [`Rooms::reset`] raises it and
    /// puts a fresh one for the threads that follow.
    stop: Mutex<Arc<AtomicBool>>,
}

impl Rooms {
    /// Nothing followed yet; `changed` hears every change.
    pub fn new(changed: Changed) -> Rooms {
        Rooms {
            spaces: Arc::new(Mutex::new(HashMap::new())),
            changed,
            stop: Mutex::new(Arc::new(AtomicBool::new(false))),
        }
    }

    /// Stops following every space: the account changed, and its streams
    /// were opened with the old one's key. Their threads end at their next
    /// event or keepalive.
    pub fn reset(&self) {
        let mut stop = self.stop.lock().expect("not poisoned");
        stop.store(true, Ordering::Relaxed);
        *stop = Arc::new(AtomicBool::new(false));
        self.spaces.lock().expect("not poisoned").clear();
        (self.changed)();
    }

    /// Starts following a space's rooms, unless it is already followed or
    /// its instance has [`MAX_PER_INSTANCE`] followed spaces.
    pub fn follow(&self, client: &Arc<Client>, space: Id) {
        let key = (client.instance().to_string(), space);
        {
            let mut spaces = self.spaces.lock().expect("not poisoned");
            let on_instance = spaces.keys().filter(|(i, _)| *i == key.0).count();
            if spaces.contains_key(&key) || on_instance >= MAX_PER_INSTANCE {
                return;
            }
            spaces.insert(key.clone(), Followed { sessions: None });
        }
        let (client, followed, changed, stop) = (
            Arc::clone(client),
            Arc::clone(&self.spaces),
            Arc::clone(&self.changed),
            Arc::clone(&self.stop.lock().expect("not poisoned")),
        );
        let thread_key = key.clone();
        let started = std::thread::Builder::new()
            .name("drywire-rooms".into())
            .spawn(move || run(&client, &thread_key, &followed, &changed, &stop));
        if started.is_err() {
            // No thread, no names: the rooms show without them.
            self.spaces.lock().expect("not poisoned").remove(&key);
        }
    }

    /// Who sits in a room, sorted by name; `None` when unknown: the space
    /// is not followed, or its stream is not open.
    pub fn occupants(&self, instance: &str, space: &Id, room: &Id) -> Option<Vec<String>> {
        let spaces = self.spaces.lock().expect("not poisoned");
        let sessions = spaces
            .get(&(instance.to_string(), *space))?
            .sessions
            .as_ref()?;
        let mut names: Vec<String> = sessions
            .values()
            .filter(|p| p.room_id == *room)
            .map(|p| printable(&p.name))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        Some(names)
    }
}

impl Drop for Rooms {
    fn drop(&mut self) {
        self.stop
            .lock()
            .expect("not poisoned")
            .store(true, Ordering::Relaxed);
    }
}

/// A stream's thread: opens the space's stream, applies its events, and
/// opens it again whenever it ends, until stopped or no longer a member.
fn run(
    client: &Client,
    key: &SpaceKey,
    spaces: &Mutex<HashMap<SpaceKey, Followed>>,
    changed: &Changed,
    stop: &AtomicBool,
) {
    let set = |sessions: Option<HashMap<u32, Presence>>| {
        if let Some(followed) = spaces.lock().expect("not poisoned").get_mut(key) {
            followed.sessions = sessions;
        }
        changed();
    };
    let mut pause = RETRY_FIRST;
    while !stop.load(Ordering::Relaxed) {
        match client.events(&key.1) {
            Ok(events) => {
                pause = RETRY_FIRST;
                // The stream ends after its first error (`Events`): a
                // broken or silent connection, or a bad event.
                for event in events.flatten() {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let mut all = spaces.lock().expect("not poisoned");
                    let Some(followed) = all.get_mut(key) else {
                        return;
                    };
                    let moved = apply(&mut followed.sessions, event);
                    drop(all);
                    if moved {
                        changed();
                    }
                }
                // Ended: whoever comes or goes now is missed until the next
                // snapshot, so the names are unknown, not shown as current.
                set(None);
            }
            // No longer a member, or the space is gone: forget it.
            Err(e) if e.status() == Some(404) => {
                spaces.lock().expect("not poisoned").remove(key);
                changed();
                return;
            }
            // Unknown until the stream opens again.
            Err(_) => {
                set(None);
                pause = (pause * 2).min(RETRY_MAX);
            }
        }
        std::thread::sleep(pause);
    }
}

/// Applies one event to a space's sessions; returns whether anything
/// changed. Events before the first snapshot are ignored: the snapshot
/// covers them.
fn apply(sessions: &mut Option<HashMap<u32, Presence>>, event: Event) -> bool {
    match event {
        Event::Snapshot(everyone) => {
            let fresh: HashMap<u32, Presence> =
                everyone.into_iter().map(|p| (p.session, p)).collect();
            let moved = sessions.as_ref() != Some(&fresh);
            *sessions = Some(fresh);
            moved
        }
        Event::Join(p) => sessions
            .as_mut()
            .is_some_and(|s| s.insert(p.session, p.clone()).as_ref() != Some(&p)),
        Event::Leave(p) | Event::Disconnect(p) => sessions
            .as_mut()
            .is_some_and(|s| s.remove(&p.session).is_some()),
        Event::Other(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drywire_control::Hex;

    fn presence(session: u32, room: u8, name: &str) -> Presence {
        Presence {
            session,
            room_id: Hex([room; 16]),
            public_key: Hex([session as u8; 32]),
            name: name.into(),
        }
    }

    #[test]
    fn events_keep_each_room_up_to_date() {
        let mut sessions = None;
        assert!(!apply(&mut sessions, Event::Join(presence(1, 1, "kim"))));
        assert_eq!(sessions, None, "nothing known before the snapshot");

        let snapshot = vec![presence(1, 1, "kim"), presence(2, 2, "lee")];
        assert!(apply(&mut sessions, Event::Snapshot(snapshot.clone())));
        assert!(!apply(&mut sessions, Event::Snapshot(snapshot)), "same");
        assert!(apply(&mut sessions, Event::Join(presence(3, 1, "Ana"))));
        assert!(!apply(&mut sessions, Event::Join(presence(3, 1, "Ana"))));
        // A leave names nobody: the session is enough.
        assert!(apply(&mut sessions, Event::Leave(presence(2, 2, ""))));
        assert!(!apply(&mut sessions, Event::Disconnect(presence(2, 2, ""))));
        assert!(!apply(&mut sessions, Event::Other("later".into())));
        let mut left: Vec<_> = sessions.unwrap().into_values().map(|p| p.name).collect();
        left.sort();
        assert_eq!(left, ["Ana", "kim"]);
    }

    #[test]
    fn an_instance_has_at_most_six_spaces_followed() {
        // Nothing listens on these ports: each stream fails to open and
        // its names stay unknown, but the space is followed all the same.
        let client = |port: u16| {
            let identity = drywire_proto::crypto::SigningKey::from_bytes(&[1; 32]);
            Arc::new(Client::new(&format!("http://127.0.0.1:{port}"), identity).unwrap())
        };
        let (here, there) = (client(9), client(7));
        let rooms = Rooms::new(Arc::new(|| {}));
        for space in 0..8 {
            rooms.follow(&here, Hex([space; 16]));
        }
        rooms.follow(&here, Hex([0; 16]));
        rooms.follow(&there, Hex([9; 16]));
        let mut followed: Vec<(String, u8)> = rooms
            .spaces
            .lock()
            .unwrap()
            .keys()
            .map(|(instance, id)| (instance.clone(), id.0[0]))
            .collect();
        followed.sort();
        let on = |instance: &Client, ids: &[u8]| {
            ids.iter()
                .map(|&id| (instance.instance().to_string(), id))
                .collect::<Vec<_>>()
        };
        let mut expected = on(&here, &[0, 1, 2, 3, 4, 5]);
        expected.extend(on(&there, &[9]));
        expected.sort();
        assert_eq!(
            followed, expected,
            "the first six here, once each; another instance has its own six"
        );
    }

    #[test]
    fn names_are_sorted_and_printable() {
        let rooms = Rooms::new(Arc::new(|| {}));
        let (instance, space) = ("http://127.0.0.1:10050".to_string(), Hex([7; 16]));
        let sessions = [
            presence(1, 1, "lee"),
            presence(2, 1, "Ana\u{7}"),
            presence(3, 2, "kim"),
        ]
        .into_iter()
        .map(|p| (p.session, p))
        .collect();
        rooms.spaces.lock().unwrap().insert(
            (instance.clone(), space),
            Followed {
                sessions: Some(sessions),
            },
        );
        assert_eq!(
            rooms.occupants(&instance, &space, &Hex([1; 16])),
            Some(vec!["Ana?".to_string(), "lee".to_string()])
        );
        assert_eq!(
            rooms.occupants(&instance, &space, &Hex([9; 16])),
            Some(vec![])
        );
        assert_eq!(
            rooms.occupants(&instance, &Hex([8; 16]), &Hex([1; 16])),
            None
        );
    }
}
