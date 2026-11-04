//! Keeping a space's MLS group (ADR 0052) in step on this device: the
//! one loop the apps run, so that people never deal with keys.
//!
//! Each round catches up with the commits the delivery service ordered,
//! then does what any member's device does for the others: let in the
//! waiting devices of members, and take out every device that is no
//! longer a member's. A device not in the group yet joins from its
//! welcome, or asks to join; the owner's first device makes the group.

use drywire_mls::{Device, Group, PendingDevice};

use crate::{Client, Error, Id, Member, PendingKeyPackage};

/// Where this device stands in a space's group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupState {
    /// It asked to join and waits for a member's device to let it in.
    Waiting,
    /// It is in, at this epoch: the room keys come from it.
    Ready(u64),
}

/// One space's group on this device.
pub struct SpaceGroup {
    space: Id,
    group: Option<Group>,
    asked: bool,
}

/// The group's errors, as the control plane's.
fn mls(e: drywire_mls::Error) -> Error {
    Error::Protocol(format!("space group: {e}"))
}

/// Whether a device key is one of the members'.
fn member_device(members: &[Member], device: &[u8; 32]) -> bool {
    members
        .iter()
        .any(|m| m.devices.iter().any(|k| k.0 == *device))
}

impl SpaceGroup {
    /// The group of `space`, as this device's state holds it, if at all.
    pub fn new(space: Id, device: &Device) -> Result<SpaceGroup, Error> {
        let group = Group::load(device, space.0).map_err(mls)?;
        Ok(SpaceGroup {
            space,
            group,
            asked: false,
        })
    }

    /// The group's epoch, if this device is in it.
    pub fn epoch(&self) -> Option<u64> {
        self.group.as_ref().map(Group::epoch)
    }

    /// The key of a room in the current epoch, if this device is in.
    pub fn room_key(&self, device: &Device, room: &Id) -> Option<[u8; 32]> {
        self.group.as_ref()?.room_key(device, &room.0).ok()
    }

    /// One round with the delivery service. `owner` says whether this
    /// account owns the space, which makes the group if nobody has. The
    /// caller saves the device's state after.
    pub fn sync(
        &mut self,
        client: &Client,
        device: &Device,
        owner: bool,
    ) -> Result<GroupState, Error> {
        let members = client.space(&self.space)?.members;
        let is_member = |d: &[u8; 32]| member_device(&members, d);
        if self.group.is_none() {
            self.enter(client, device, owner, &is_member)?;
        }
        let Some(group) = self.group.as_mut() else {
            return Ok(GroupState::Waiting);
        };
        loop {
            // Catch up, 64 commits at a time.
            loop {
                let commits = client.commits(&self.space, group.epoch())?;
                if commits.is_empty() {
                    break;
                }
                for (_, commit) in commits {
                    if group.apply(device, &commit, is_member).is_err() {
                        // Removed, or a state gone wrong: start again as a
                        // new leaf.
                        self.leave(device)?;
                        return self.sync(client, device, owner);
                    }
                }
            }
            if !group.is_active() {
                self.leave(device)?;
                return self.sync(client, device, owner);
            }
            let staged = match group.prune(device, is_member).map_err(mls)? {
                Some(commit) => Some(commit),
                None => {
                    let waiting: Vec<PendingDevice> = client
                        .key_packages(&self.space)?
                        .into_iter()
                        .filter(|p| is_member(&p.device))
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
                    if waiting.is_empty() {
                        None
                    } else {
                        group.add(device, &waiting, is_member).ok()
                    }
                }
            };
            let Some(commit) = staged else {
                return Ok(GroupState::Ready(group.epoch()));
            };
            match client.post_commit(
                &self.space,
                group.epoch(),
                &commit.commit,
                commit.welcome.as_deref(),
                &commit.welcomed,
            ) {
                Ok(_) => group.confirm(device).map_err(mls)?,
                // Another member's commit came first: catch up and see
                // whether there is still something to do.
                Err(Error::StaleEpoch { .. }) => group.abandon(device).map_err(mls)?,
                Err(e) => {
                    group.abandon(device).map_err(mls)?;
                    return Err(e);
                }
            }
        }
    }

    /// Joins from a welcome, makes the group, or asks to join.
    fn enter(
        &mut self,
        client: &Client,
        device: &Device,
        owner: bool,
        is_member: &impl Fn(&[u8; 32]) -> bool,
    ) -> Result<(), Error> {
        let me = client.public_key();
        if let Some((_, welcome)) = client.welcome(&self.space, &me)?
            && let Ok(group) = Group::join(device, &welcome, is_member)
        {
            self.group = Some(group);
            return Ok(());
        }
        if owner && client.commits(&self.space, 0)?.is_empty() {
            // The group exists once the service took its first commit:
            // two devices of the owner may try at once, and one wins.
            let mut group = Group::create(device, self.space.0).map_err(mls)?;
            let commit = group.renew(device).map_err(mls)?;
            match client.post_commit(&self.space, 0, &commit.commit, None, &[]) {
                Ok(_) => {
                    group.confirm(device).map_err(mls)?;
                    self.group = Some(group);
                    return Ok(());
                }
                Err(Error::StaleEpoch { .. }) => group.forget(device).map_err(mls)?,
                Err(e) => {
                    group.forget(device).map_err(mls)?;
                    return Err(e);
                }
            }
        }
        if !self.asked {
            let package = device.key_package().map_err(mls)?;
            client.post_key_package(&self.space, &me, &package)?;
            self.asked = true;
        }
        Ok(())
    }

    /// Forgets the group, to ask again.
    fn leave(&mut self, device: &Device) -> Result<(), Error> {
        if let Some(group) = self.group.take() {
            group.forget(device).map_err(mls)?;
        }
        self.asked = false;
        Ok(())
    }
}
