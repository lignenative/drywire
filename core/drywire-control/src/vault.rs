//! Account vaults on the control plane (protocol section 15, ADR 0051):
//! what goes inside one, sealing it for an instance, and opening one with
//! a code on a device that has no account yet.

use drywire_proto::crypto::SigningKey;
use drywire_proto::vault::{ID_SIZE, SECRET_SIZE, VaultCode, VaultError};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Client, Error, Hex, Id, Method, to_hex};

/// What a vault holds: the account and its spaces (section 15.2).
#[derive(Clone, Serialize, Deserialize)]
pub struct VaultContents {
    /// The account's Ed25519 seed.
    pub account: Hex<32>,
    /// The display name.
    pub name: String,
    /// The spaces, with their keys.
    pub spaces: Vec<VaultSpace>,
}

/// One space in a vault.
#[derive(Clone, Serialize, Deserialize)]
pub struct VaultSpace {
    /// The instance's canonical origin.
    pub instance: String,
    /// The space's id.
    pub id: Id,
    /// Its name, as last seen.
    pub name: String,
    /// The space key.
    pub key: Hex<32>,
}

impl std::fmt::Debug for VaultContents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keys are secrets: never in a log by accident.
        write!(f, "VaultContents({} spaces)", self.spaces.len())
    }
}

impl VaultContents {
    /// The account's signing key.
    pub fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.account.0)
    }

    /// Sealed with `code`, under a fresh random nonce.
    pub fn seal(&self, code: &VaultCode) -> Vec<u8> {
        let plaintext = serde_json::to_vec(self).expect("a vault always serialises");
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).expect("the system gives random bytes");
        code.seal(&plaintext, nonce)
    }
}

/// What a vault is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultKind {
    /// Opened with a recovery code.
    Recovery,
    /// Opened with a device code, which now opens nothing.
    Device,
}

/// A device code, waiting on an instance.
#[derive(Debug, Deserialize)]
pub struct DeviceVault {
    /// Seconds until it stops working.
    pub expires_in: u64,
}

/// A new code, from the system's random bytes.
pub fn new_code() -> VaultCode {
    let mut secret = [0u8; SECRET_SIZE];
    getrandom::fill(&mut secret).expect("the system gives random bytes");
    VaultCode::from_secret(secret)
}

/// Opens the vault a code names on an instance, for a device that has no
/// account yet: no login. Every way to get it wrong (an unknown code, one
/// already used, a vault gone) is the same refusal.
pub fn open_vault(instance: &str, code: &str) -> Result<(VaultKind, VaultContents), Error> {
    #[derive(Deserialize)]
    struct Opened {
        kind: VaultKind,
        sealed: String,
    }
    let code = VaultCode::parse(code).map_err(vault_error)?;
    // The client's key is never used: this request needs no login.
    let client = Client::new(instance, SigningKey::from_bytes(&[0; 32]))?;
    let opened: Opened = client
        .send(
            Method::Post,
            "/v1/vaults/open",
            Some(&json!({ "id": to_hex(&code.id()) })),
            None,
        )
        .map_err(|e| match e.status() {
            Some(404) => Error::Refused {
                status: 404,
                message: "this code is unknown, used or expired on this instance".into(),
            },
            _ => e,
        })?;
    let sealed = crate::from_hex(&opened.sealed)
        .ok_or_else(|| Error::Protocol("the vault is not hex".into()))?;
    let plaintext = code.open(&sealed).map_err(vault_error)?;
    let contents: VaultContents =
        serde_json::from_slice(&plaintext).map_err(|e| Error::Protocol(e.to_string()))?;
    Ok((opened.kind, contents))
}

fn vault_error(e: VaultError) -> Error {
    Error::BadCode(e.to_string())
}

impl Client {
    /// Keeps this account's recovery vault on the instance, sealed with
    /// `code`, replacing the one it had.
    pub fn put_recovery_vault(
        &self,
        code: &VaultCode,
        contents: &VaultContents,
    ) -> Result<(), Error> {
        let body = vault_body(code, contents);
        self.call(Method::Put, "/v1/me/vault", Some(&body))
    }

    /// Deletes this account's recovery vault from the instance.
    pub fn delete_recovery_vault(&self) -> Result<(), Error> {
        self.call(Method::Delete, "/v1/me/vault", None)
    }

    /// Leaves a vault on the instance for one new device, for ten minutes.
    pub fn put_device_vault(
        &self,
        code: &VaultCode,
        contents: &VaultContents,
    ) -> Result<DeviceVault, Error> {
        let body = vault_body(code, contents);
        self.call(Method::Post, "/v1/me/device-vaults", Some(&body))
    }
}

fn vault_body(code: &VaultCode, contents: &VaultContents) -> serde_json::Value {
    let id: [u8; ID_SIZE] = code.id();
    json!({ "id": to_hex(&id), "sealed": to_hex(&contents.seal(code)) })
}
