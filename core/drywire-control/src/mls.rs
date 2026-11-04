//! The space's MLS delivery service (ADR 0052, protocol section 16): the
//! bytes drywire-mls makes and reads, carried as hex.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Client, Error, Id, Method, PublicKey, from_hex, refused, to_hex};

/// A device waiting to join a space's group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingKeyPackage {
    /// The device's key.
    pub device: [u8; 32],
    /// Its key package.
    pub key_package: Vec<u8>,
}

fn bytes(value: &Value, field: &str) -> Result<Vec<u8>, Error> {
    value[field]
        .as_str()
        .and_then(from_hex)
        .ok_or_else(|| Error::Protocol(format!("{field}: not hex")))
}

impl Client {
    /// Posts this device's key package: it waits to be added to the group.
    pub fn post_key_package(
        &self,
        space: &Id,
        device: &PublicKey,
        key_package: &[u8],
    ) -> Result<(), Error> {
        let body = json!({ "device": device, "key_package": to_hex(key_package) });
        let path = format!("/v1/spaces/{space}/mls/key-packages");
        let _: Value = self.call(Method::Post, &path, Some(&body))?;
        Ok(())
    }

    /// The devices waiting to join.
    pub fn key_packages(&self, space: &Id) -> Result<Vec<PendingKeyPackage>, Error> {
        let list: Vec<Value> = self.call(
            Method::Get,
            &format!("/v1/spaces/{space}/mls/key-packages"),
            None,
        )?;
        list.iter()
            .map(|v| {
                let device = bytes(v, "device")?
                    .try_into()
                    .map_err(|_| Error::Protocol("device: 32 bytes".into()))?;
                Ok(PendingKeyPackage {
                    device,
                    key_package: bytes(v, "key_package")?,
                })
            })
            .collect()
    }

    /// Posts a commit made in `epoch`, with the welcome for the devices it
    /// adds; the epoch it started, or [`Error::StaleEpoch`].
    pub fn post_commit(
        &self,
        space: &Id,
        epoch: u64,
        commit: &[u8],
        welcome: Option<&[u8]>,
        welcomed: &[[u8; 32]],
    ) -> Result<u64, Error> {
        let body = json!({
            "epoch": epoch,
            "commit": to_hex(commit),
            "welcome": welcome.map(to_hex).unwrap_or_default(),
            "welcomed": welcomed.iter().map(|d| to_hex(d)).collect::<Vec<_>>(),
        });
        let path = format!("/v1/spaces/{space}/mls/commits");
        match self.call_value(Method::Post, &path, Some(&body))? {
            (409, value) => Err(Error::StaleEpoch {
                current: value["epoch"].as_u64().unwrap_or(0),
            }),
            (status, value) if status >= 400 => Err(refused(status, &value)),
            (_, value) => value["epoch"]
                .as_u64()
                .ok_or_else(|| Error::Protocol("no epoch".into())),
        }
    }

    /// The commits that started the epochs after `after`, in order, 64 at
    /// most: ask again from the last one for more.
    pub fn commits(&self, space: &Id, after: u64) -> Result<Vec<(u64, Vec<u8>)>, Error> {
        #[derive(Deserialize)]
        struct Row {
            epoch: u64,
            commit: String,
        }
        let rows: Vec<Row> = self.call(
            Method::Get,
            &format!("/v1/spaces/{space}/mls/commits?after={after}"),
            None,
        )?;
        rows.into_iter()
            .map(|r| {
                let commit =
                    from_hex(&r.commit).ok_or_else(|| Error::Protocol("commit: not hex".into()))?;
                Ok((r.epoch, commit))
            })
            .collect()
    }

    /// The welcome kept for this device, and the epoch it joins, if any.
    pub fn welcome(&self, space: &Id, device: &PublicKey) -> Result<Option<(u64, Vec<u8>)>, Error> {
        let path = format!("/v1/spaces/{space}/mls/welcome?device={device}");
        match self.call::<Value>(Method::Get, &path, None) {
            Ok(v) => Ok(Some((
                v["epoch"]
                    .as_u64()
                    .ok_or_else(|| Error::Protocol("no epoch".into()))?,
                bytes(&v, "welcome")?,
            ))),
            Err(e) if e.status() == Some(404) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
