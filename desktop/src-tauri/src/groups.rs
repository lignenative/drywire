//! The spaces' MLS groups on this device (ADRs 0052 and 0055): one state
//! for all of them, saved sealed after every round, and one loop per
//! space that the calls run. Nobody ever sees a key.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drywire_control::{Client, GroupState, Id, SpaceGroup};
use drywire_mls::Device;

use crate::error::AppError;
use crate::store::Store;

/// How long a join waits for a member's app to let this device in.
const LET_IN_WAIT: Duration = Duration::from_secs(120);

struct Inner {
    device: Option<Device>,
    spaces: HashMap<(String, Id), SpaceGroup>,
}

/// The groups of every known space.
pub struct Groups {
    store: Arc<Store>,
    inner: Mutex<Inner>,
}

impl Groups {
    /// Groups saved in `store`.
    pub fn new(store: Arc<Store>) -> Groups {
        Groups {
            store,
            inner: Mutex::new(Inner {
                device: None,
                spaces: HashMap::new(),
            }),
        }
    }

    /// One round for a space; saves the state after it.
    pub fn sync(&self, client: &Client, space: &Id, owner: bool) -> Result<GroupState, AppError> {
        let mut inner = self.inner.lock().expect("not poisoned");
        let inner = &mut *inner;
        if inner.device.is_none() {
            inner.device = Some(self.store.group_device().map_err(AppError::storage)?);
        }
        let device = inner.device.as_ref().expect("loaded");
        let key = (client.instance().to_string(), *space);
        if !inner.spaces.contains_key(&key) {
            let group = SpaceGroup::new(*space, device)?;
            inner.spaces.insert(key.clone(), group);
        }
        let group = inner.spaces.get_mut(&key).expect("inserted");
        let state = group.sync(client, device, owner);
        self.store
            .save_group_device(device)
            .map_err(AppError::storage)?;
        Ok(state?)
    }

    /// The key of a room in the space's current epoch, if this device is
    /// in its group.
    pub fn room_key(&self, instance: &str, space: &Id, room: &Id) -> Option<[u8; 32]> {
        let inner = self.inner.lock().expect("not poisoned");
        let device = inner.device.as_ref()?;
        inner
            .spaces
            .get(&(instance.to_string(), *space))?
            .room_key(device, room)
    }

    /// Syncs until this device is in the space's group, for a join: a
    /// member's app may have to let it in. Gives up when `stopped` says so.
    pub fn ready(
        &self,
        client: &Client,
        space: &Id,
        room: &Id,
        owner: bool,
        stopped: &dyn Fn() -> bool,
    ) -> Result<[u8; 32], AppError> {
        let started = Instant::now();
        loop {
            if let GroupState::Ready(_) = self.sync(client, space, owner)? {
                return self
                    .room_key(client.instance(), space, room)
                    .ok_or_else(|| {
                        AppError::new("group", "this device is not in the space's group")
                    });
            }
            if started.elapsed() > LET_IN_WAIT {
                return Err(AppError::new(
                    "group_waiting",
                    "no member's app let this device into the space yet: try again when one is online",
                ));
            }
            if stopped() {
                return Err(AppError::new("stopped", "the call was left"));
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}
