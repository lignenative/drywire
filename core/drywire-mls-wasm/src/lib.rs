//! drywire-mls for the web client (ADRs 0052 and 0055): this browser in
//! the spaces' MLS groups. The page carries the bytes to and from the
//! delivery service and decides when to sync, as drywire-control's
//! SpaceGroup does natively; this module only keeps the groups and makes
//! or reads their messages.
//!
//! Lists of device keys cross as one byte array, 32 bytes per key.

use drywire_mls::{Commit, Device, Group, PendingDevice};
use ed25519_dalek::SigningKey;
use wasm_bindgen::prelude::*;

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn keys(flat: &[u8]) -> Vec<[u8; 32]> {
    flat.as_chunks::<32>().0.to_vec()
}

fn id16(bytes: &[u8]) -> Result<[u8; 16], JsError> {
    bytes
        .try_into()
        .map_err(|_| JsError::new("an id is 16 bytes"))
}

/// This browser in the groups: the key it logs in with, and the groups'
/// state, which the page saves.
#[wasm_bindgen]
pub struct WebDevice {
    device: Device,
}

#[wasm_bindgen]
impl WebDevice {
    /// A device signing with the 32-byte seed of the key it logs in with.
    #[wasm_bindgen(constructor)]
    pub fn new(seed: &[u8]) -> Result<WebDevice, JsError> {
        let seed: [u8; 32] = seed
            .try_into()
            .map_err(|_| JsError::new("a seed is 32 bytes"))?;
        Ok(WebDevice {
            device: Device::new(&SigningKey::from_bytes(&seed)).map_err(err)?,
        })
    }

    /// A device saved by [`WebDevice::save`].
    pub fn load(state: &[u8]) -> Result<WebDevice, JsError> {
        Ok(WebDevice {
            device: Device::load(state).map_err(err)?,
        })
    }

    /// The device's public key.
    #[wasm_bindgen(js_name = publicKey)]
    pub fn public_key(&self) -> Vec<u8> {
        self.device.public_key().to_vec()
    }

    /// Everything to keep, as secret as the identity.
    pub fn save(&self) -> Vec<u8> {
        self.device.save()
    }

    /// A key package to post for a space this browser wants to join.
    #[wasm_bindgen(js_name = keyPackage)]
    pub fn key_package(&self) -> Result<Vec<u8>, JsError> {
        self.device.key_package().map_err(err)
    }
}

/// A staged commit, to post.
#[wasm_bindgen(getter_with_clone)]
pub struct StagedCommit {
    /// The commit, for every member.
    pub commit: Vec<u8>,
    /// Empty when the commit adds nobody.
    pub welcome: Vec<u8>,
    /// The devices the welcome is for, 32 bytes each.
    pub welcomed: Vec<u8>,
}

impl From<Commit> for StagedCommit {
    fn from(c: Commit) -> StagedCommit {
        StagedCommit {
            commit: c.commit,
            welcome: c.welcome.unwrap_or_default(),
            welcomed: c.welcomed.concat(),
        }
    }
}

/// One space's group in this browser.
#[wasm_bindgen]
pub struct WebGroup {
    group: Group,
}

#[wasm_bindgen]
impl WebGroup {
    /// A new group for `space`, this browser its only leaf.
    pub fn create(device: &WebDevice, space: &[u8]) -> Result<WebGroup, JsError> {
        Ok(WebGroup {
            group: Group::create(&device.device, id16(space)?).map_err(err)?,
        })
    }

    /// The group of `space` in the saved state, if this browser is in it.
    pub fn load(device: &WebDevice, space: &[u8]) -> Result<Option<WebGroup>, JsError> {
        Ok(Group::load(&device.device, id16(space)?)
            .map_err(err)?
            .map(|group| WebGroup { group }))
    }

    /// Joins from a welcome; every leaf must be one of `members`' devices.
    pub fn join(device: &WebDevice, welcome: &[u8], members: &[u8]) -> Result<WebGroup, JsError> {
        let members = keys(members);
        let group = Group::join(&device.device, welcome, |d| members.contains(d)).map_err(err)?;
        Ok(WebGroup { group })
    }

    /// The epoch, as the delivery service numbers it.
    pub fn epoch(&self) -> f64 {
        self.group.epoch() as f64
    }

    /// Whether this browser is still in the group.
    #[wasm_bindgen(js_name = isActive)]
    pub fn is_active(&self) -> bool {
        self.group.is_active()
    }

    /// Applies another member's commit.
    pub fn apply(
        &mut self,
        device: &WebDevice,
        commit: &[u8],
        members: &[u8],
    ) -> Result<(), JsError> {
        let members = keys(members);
        self.group
            .apply(&device.device, commit, |d| members.contains(d))
            .map_err(err)
    }

    /// Stages the removal of every leaf no longer one of `members`'
    /// devices; undefined when there is none.
    pub fn prune(
        &mut self,
        device: &WebDevice,
        members: &[u8],
    ) -> Result<Option<StagedCommit>, JsError> {
        let members = keys(members);
        Ok(self
            .group
            .prune(&device.device, |d| members.contains(d))
            .map_err(err)?
            .map(Into::into))
    }

    /// Stages adding waiting devices: their keys, 32 bytes each, and their
    /// key packages one after the other, `lengths` saying where each ends.
    pub fn add(
        &mut self,
        device: &WebDevice,
        devices: &[u8],
        packages: &[u8],
        lengths: &[u32],
        members: &[u8],
    ) -> Result<StagedCommit, JsError> {
        let members = keys(members);
        let mut rest = packages;
        let mut pending = Vec::new();
        for (device, &len) in keys(devices).into_iter().zip(lengths) {
            let (package, tail) = rest
                .split_at_checked(len as usize)
                .ok_or_else(|| JsError::new("lengths past the packages"))?;
            pending.push(PendingDevice {
                device,
                key_package: package.to_vec(),
            });
            rest = tail;
        }
        Ok(self
            .group
            .add(&device.device, &pending, |d| members.contains(d))
            .map_err(err)?
            .into())
    }

    /// Stages renewing this browser's own leaf.
    pub fn renew(&mut self, device: &WebDevice) -> Result<StagedCommit, JsError> {
        Ok(self.group.renew(&device.device).map_err(err)?.into())
    }

    /// The service took the staged commit.
    pub fn confirm(&mut self, device: &WebDevice) -> Result<(), JsError> {
        self.group.confirm(&device.device).map_err(err)
    }

    /// Another commit came first.
    pub fn abandon(&mut self, device: &WebDevice) -> Result<(), JsError> {
        self.group.abandon(&device.device).map_err(err)
    }

    /// Forgets the group in this browser, to ask again.
    pub fn forget(self, device: &WebDevice) -> Result<(), JsError> {
        self.group.forget(&device.device).map_err(err)
    }

    /// The key of `room` in the current epoch.
    #[wasm_bindgen(js_name = roomKey)]
    pub fn room_key(&self, device: &WebDevice, room: &[u8]) -> Result<Vec<u8>, JsError> {
        Ok(self
            .group
            .room_key(&device.device, &id16(room)?)
            .map_err(err)?
            .to_vec())
    }
}
