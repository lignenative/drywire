//! One space's group, as this device sees it.
//!
//! Making a commit is two steps, because the delivery service orders
//! them: [`Group::add`], [`Group::remove`] or [`Group::renew`] stage a
//! commit, the app posts it, then calls [`Group::confirm`] if the service
//! took it, or [`Group::abandon`] if another member's commit came first
//! (`409`), applies what it missed with [`Group::apply`], and tries again.

use openmls::prelude::{
    tls_codec::{Deserialize as _, Serialize as _},
    *,
};

use crate::{CIPHERSUITE, Device, Error, credential, mls};

/// A device that waits to join, as the delivery service lists it.
pub struct PendingDevice {
    /// The device's signature key, which it posted the key package for.
    pub device: [u8; 32],
    /// The account the control plane says posted it.
    pub account: [u8; 32],
    /// The key package, as posted.
    pub key_package: Vec<u8>,
}

/// A staged commit, to post to the delivery service.
pub struct Commit {
    /// The commit, for every member.
    pub commit: Vec<u8>,
    /// The welcome for the devices added, if any: one message for all of
    /// them, which the service hands to each.
    pub welcome: Option<Vec<u8>>,
    /// The devices the welcome is for.
    pub welcomed: Vec<[u8; 32]>,
}

/// One leaf of the group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    /// Its index in the tree.
    pub index: u32,
    /// The account it belongs to.
    pub account: [u8; 32],
    /// The device's signature key.
    pub device: [u8; 32],
}

/// One space's group.
pub struct Group {
    inner: MlsGroup,
}

impl Group {
    /// A new group for `space`, with this device as its only leaf: the
    /// space's creator, or its owner moving it to MLS.
    pub fn create(device: &Device, space: [u8; 16]) -> Result<Group, Error> {
        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(CIPHERSUITE)
            .use_ratchet_tree_extension(true)
            .build();
        let inner = MlsGroup::new_with_group_id(
            &device.provider,
            &device.signer,
            &config,
            GroupId::from_slice(&space),
            device.credential.clone(),
        )
        .map_err(mls)?;
        Ok(Group { inner })
    }

    /// The group of `space` this device is in, from its saved state.
    pub fn load(device: &Device, space: [u8; 16]) -> Result<Option<Group>, Error> {
        let group =
            MlsGroup::load(device.provider.storage(), &GroupId::from_slice(&space)).map_err(mls)?;
        Ok(group.map(|inner| Group { inner }))
    }

    /// Joins a group from the welcome the delivery service kept for this
    /// device. Every leaf must be certified by its account, and that
    /// account a member of the space, as `is_member` says from the control
    /// plane's list.
    pub fn join(
        device: &Device,
        welcome: &[u8],
        is_member: impl Fn(&[u8; 32]) -> bool,
    ) -> Result<Group, Error> {
        let message = MlsMessageIn::tls_deserialize_exact(welcome).map_err(|_| Error::Malformed)?;
        let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
            return Err(Error::Malformed);
        };
        let config = MlsGroupJoinConfig::builder()
            .use_ratchet_tree_extension(true)
            .build();
        let staged = StagedWelcome::new_from_welcome(&device.provider, &config, welcome, None)
            .map_err(mls)?;
        for member in staged.members() {
            check(&member.credential, &member.signature_key, &is_member)?;
        }
        let inner = staged.into_group(&device.provider).map_err(mls)?;
        Ok(Group { inner })
    }

    /// The space this group is for.
    pub fn space(&self) -> [u8; 16] {
        self.inner
            .group_id()
            .as_slice()
            .try_into()
            .expect("a space's group id is its 16-byte id")
    }

    /// The group's epoch, as the delivery service numbers it.
    pub fn epoch(&self) -> u64 {
        self.inner.epoch().as_u64()
    }

    /// The group's leaves. Each was checked when it came in.
    pub fn leaves(&self) -> Vec<Leaf> {
        self.inner
            .members()
            .filter_map(|m| {
                let identity = BasicCredential::try_from(m.credential).ok()?;
                Some(Leaf {
                    index: m.index.u32(),
                    account: credential::verify(identity.identity(), &m.signature_key)?,
                    device: m.signature_key.try_into().ok()?,
                })
            })
            .collect()
    }

    /// Stages a commit that adds waiting devices. Each key package must be
    /// signed by the device it was posted for and certified by the account
    /// that posted it; the caller lists only members' devices.
    pub fn add(&mut self, device: &Device, pending: &[PendingDevice]) -> Result<Commit, Error> {
        let mut packages = Vec::with_capacity(pending.len());
        for p in pending {
            let package = KeyPackageIn::tls_deserialize_exact(p.key_package.as_slice())
                .map_err(|_| Error::Malformed)?
                .validate(device.provider.crypto(), ProtocolVersion::Mls10)
                .map_err(mls)?;
            // One suite only: the others' code is not ours to vouch for
            // (core/deny.toml).
            if package.ciphersuite() != CIPHERSUITE {
                return Err(Error::Malformed);
            }
            let leaf = package.leaf_node();
            if leaf.signature_key().as_slice() != p.device {
                return Err(Error::WrongDevice);
            }
            let account = check(
                leaf.credential(),
                leaf.signature_key().as_slice(),
                &|_: &[u8; 32]| true,
            )?;
            if account != p.account {
                return Err(Error::WrongDevice);
            }
            packages.push(package);
        }
        let (commit, welcome, _) = self
            .inner
            .add_members(&device.provider, &device.signer, &packages)
            .map_err(mls)?;
        Ok(Commit {
            commit: commit.tls_serialize_detached().map_err(mls)?,
            welcome: Some(welcome.tls_serialize_detached().map_err(mls)?),
            welcomed: pending.iter().map(|p| p.device).collect(),
        })
    }

    /// Stages a commit that removes every device of these accounts: a
    /// member who left, was removed or banned. Nothing to remove gives
    /// `None`. This device never removes itself.
    pub fn remove(
        &mut self,
        device: &Device,
        accounts: &[[u8; 32]],
    ) -> Result<Option<Commit>, Error> {
        let own = self.inner.own_leaf_index();
        let leaves: Vec<_> = self
            .leaves()
            .into_iter()
            .filter(|l| accounts.contains(&l.account) && l.index != own.u32())
            .map(|l| LeafNodeIndex::new(l.index))
            .collect();
        if leaves.is_empty() {
            return Ok(None);
        }
        let (commit, _, _) = self
            .inner
            .remove_members(&device.provider, &device.signer, &leaves)
            .map_err(mls)?;
        Ok(Some(Commit {
            commit: commit.tls_serialize_detached().map_err(mls)?,
            welcome: None,
            welcomed: vec![],
        }))
    }

    /// Stages a commit that renews this device's own key material, for
    /// post-compromise security: done at least daily while the app runs.
    pub fn renew(&mut self, device: &Device) -> Result<Commit, Error> {
        let bundle = self
            .inner
            .self_update(
                &device.provider,
                &device.signer,
                LeafNodeParameters::default(),
            )
            .map_err(mls)?;
        Ok(Commit {
            commit: bundle.commit().tls_serialize_detached().map_err(mls)?,
            welcome: None,
            welcomed: vec![],
        })
    }

    /// The delivery service took the staged commit: the group moves to the
    /// new epoch.
    pub fn confirm(&mut self, device: &Device) -> Result<(), Error> {
        self.inner
            .merge_pending_commit(&device.provider)
            .map_err(mls)
    }

    /// Another member's commit came first: the staged one is dropped.
    pub fn abandon(&mut self, device: &Device) -> Result<(), Error> {
        self.inner
            .clear_pending_commit(device.provider.storage())
            .map_err(mls)
    }

    /// Applies another member's commit, in the delivery service's order.
    /// Every leaf it brings in or changes must be certified by a member's
    /// account; otherwise the commit is refused and the group stays where
    /// it was.
    pub fn apply(
        &mut self,
        device: &Device,
        commit: &[u8],
        is_member: impl Fn(&[u8; 32]) -> bool,
    ) -> Result<(), Error> {
        let message = MlsMessageIn::tls_deserialize_exact(commit).map_err(|_| Error::Malformed)?;
        let message = message
            .try_into_protocol_message()
            .map_err(|_| Error::Malformed)?;
        let processed = self
            .inner
            .process_message(&device.provider, message)
            .map_err(mls)?;
        let ProcessedMessageContent::StagedCommitMessage(staged) = processed.into_content() else {
            return Err(Error::Malformed);
        };
        let leaf = |leaf: &LeafNode| {
            check(
                leaf.credential(),
                leaf.signature_key().as_slice(),
                &is_member,
            )
            .map(|_| ())
        };
        for p in staged.add_proposals() {
            leaf(p.add_proposal().key_package().leaf_node())?;
        }
        for p in staged.update_proposals() {
            leaf(p.update_proposal().leaf_node())?;
        }
        if let Some(path) = staged.update_path_leaf_node() {
            leaf(path)?;
        }
        self.inner
            .merge_staged_commit(&device.provider, *staged)
            .map_err(mls)
    }

    /// The key of `room` in the current epoch, for the audio path (ADR
    /// 0052): `MLS-Exporter("drywire/room/v1", room id, 32)`.
    pub fn room_key(&self, device: &Device, room: &[u8; 16]) -> Result<[u8; 32], Error> {
        let key = self
            .inner
            .export_secret(device.provider.crypto(), "drywire/room/v1", room, 32)
            .map_err(mls)?;
        key.try_into()
            .map_err(|_| Error::Mls("the exporter gave another length".into()))
    }
}

/// The account a leaf belongs to, if its credential certifies its
/// signature key and the account is a member.
fn check(
    credential: &Credential,
    signature_key: &[u8],
    is_member: &impl Fn(&[u8; 32]) -> bool,
) -> Result<[u8; 32], Error> {
    let identity =
        BasicCredential::try_from(credential.clone()).map_err(|_| Error::BadCredential)?;
    let account =
        credential::verify(identity.identity(), signature_key).ok_or(Error::BadCredential)?;
    if !is_member(&account) {
        return Err(Error::NotMember);
    }
    Ok(account)
}
