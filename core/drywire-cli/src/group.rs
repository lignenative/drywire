//! A space's MLS group during a call (ADRs 0052 and 0055): the room key
//! comes from it, and changes with it.

use std::time::{Duration, Instant};

use drywire_control::{Client, GroupState, Id, SpaceGroup};
use drywire_core::call::CallMembers;
use drywire_mls::Device;

use crate::home::{Home, KnownSpace};

/// How long `join` waits for a member's device to let this one in.
const LET_IN_WAIT: Duration = Duration::from_secs(120);
/// How often a call syncs the group without being told to: a member who
/// left the space says so by no event.
pub const RESYNC: Duration = Duration::from_secs(30);

/// This device's part of one space's group.
pub struct Runner {
    home: Home,
    device: Device,
    group: SpaceGroup,
    owner: bool,
    /// The room key the call has, to tell a new one.
    key: Option<[u8; 32]>,
    pub last_sync: Instant,
}

impl Runner {
    /// The group of `space`, synced until this device is in: it may have
    /// to wait for another member's app to let it in.
    pub fn ready(home: Home, client: &Client, space: &KnownSpace) -> Result<Runner, String> {
        let device = home.group_device()?;
        let group = SpaceGroup::new(space.space_id, &device).map_err(|e| e.to_string())?;
        let owner = client
            .space(&space.space_id)
            .map_err(|e| e.to_string())?
            .role
            == "owner";
        let mut runner = Runner {
            home,
            device,
            group,
            owner,
            key: None,
            last_sync: Instant::now(),
        };
        let started = Instant::now();
        let mut told = false;
        loop {
            match runner.sync(client)? {
                GroupState::Ready(_) => return Ok(runner),
                GroupState::Waiting if started.elapsed() > LET_IN_WAIT => {
                    return Err("no member's app let this device into the space yet: try again when one is online".into());
                }
                GroupState::Waiting => {
                    if !told {
                        eprintln!("waiting for a member's app to let this device in…");
                        told = true;
                    }
                    if crate::stop::requested() {
                        return Err("stopped".into());
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    fn sync(&mut self, client: &Client) -> Result<GroupState, String> {
        let state = self
            .group
            .sync(client, &self.device, self.owner)
            .map_err(|e| e.to_string());
        // Saved after every round, whatever happened: the state moved.
        self.home.save_group_device(&self.device)?;
        self.last_sync = Instant::now();
        state
    }

    /// The key of `room` now.
    pub fn room_key(&mut self, room: &Id) -> Result<[u8; 32], String> {
        let key = self
            .group
            .room_key(&self.device, room)
            .ok_or("this device is not in the space's group")?;
        self.key = Some(key);
        Ok(key)
    }

    /// Syncs, and hands the call the room's new key if the group moved.
    pub fn refresh(&mut self, client: &Client, room: &Id, call: &CallMembers) {
        if self.sync(client).is_err() {
            return; // the next event or RESYNC tries again
        }
        if let Some(key) = self.group.room_key(&self.device, room)
            && Some(key) != self.key
        {
            self.key = Some(key);
            call.set_room_key(key);
        }
    }
}
