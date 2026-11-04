//! This device's MLS identity and the state of its groups.

use ed25519_dalek::SigningKey;
use openmls::prelude::{tls_codec::Serialize as _, *};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;

use crate::{CIPHERSUITE, Error, mls};

/// The version of [`Device::save`]'s bytes.
const STATE_VERSION: u8 = 1;

/// This device in every space's group: its own signature key, and
/// OpenMLS's state for each group and pending key
/// package, which [`Device::save`] writes out whole.
pub struct Device {
    pub(crate) provider: OpenMlsRustCrypto,
    pub(crate) signer: SignatureKeyPair,
    pub(crate) credential: CredentialWithKey,
}

impl Device {
    /// This device in the groups, signing with its own key: the one it
    /// logs in with (ADR 0055).
    pub fn new(key: &SigningKey) -> Result<Device, Error> {
        let public = key.verifying_key().to_bytes();
        let signer = SignatureKeyPair::from_raw(
            CIPHERSUITE.signature_algorithm(),
            key.to_bytes().to_vec(),
            public.to_vec(),
        );
        Device::with(OpenMlsRustCrypto::default(), signer, &public)
    }

    fn with(
        provider: OpenMlsRustCrypto,
        signer: SignatureKeyPair,
        identity: &[u8],
    ) -> Result<Device, Error> {
        signer.store(provider.storage()).map_err(mls)?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(identity.to_vec()).into(),
            signature_key: signer.public().into(),
        };
        Ok(Device {
            provider,
            signer,
            credential,
        })
    }

    /// The device's signature key: how the delivery service and the group
    /// name it.
    pub fn public_key(&self) -> [u8; 32] {
        self.signer
            .public()
            .try_into()
            .expect("an Ed25519 public key is 32 bytes")
    }

    /// A key package to post for a space this device wants to join: any
    /// member's app adds it to the group. Its private part stays in this
    /// device's state until the welcome comes.
    pub fn key_package(&self) -> Result<Vec<u8>, Error> {
        let bundle = KeyPackage::builder()
            .build(
                CIPHERSUITE,
                &self.provider,
                &self.signer,
                self.credential.clone(),
            )
            .map_err(mls)?;
        bundle.key_package().tls_serialize_detached().map_err(mls)
    }

    /// Everything this device holds, for its sealed store: the signature
    /// key and every group's state. The bytes are as
    /// secret as the device's other keys.
    pub fn save(&self) -> Vec<u8> {
        let values = self
            .provider
            .storage()
            .values
            .read()
            .expect("the storage lock is never poisoned");
        let mut entries: Vec<_> = values.iter().collect();
        entries.sort();
        let identity = self.identity();
        let mut out = vec![STATE_VERSION];
        out.extend_from_slice(&self.public_key());
        put(&mut out, identity);
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (key, value) in entries {
            put(&mut out, key);
            put(&mut out, value);
        }
        out
    }

    /// A device saved by [`Device::save`].
    pub fn load(bytes: &[u8]) -> Result<Device, Error> {
        let mut r = Reader(bytes);
        if r.take(1)? != [STATE_VERSION] {
            return Err(Error::BadState);
        }
        let public = r.take(32)?.to_vec();
        let identity = r.chunk()?.to_vec();
        let count = u32::from_be_bytes(r.take(4)?.try_into().map_err(|_| Error::BadState)?);
        let provider = OpenMlsRustCrypto::default();
        {
            let mut values = provider
                .storage()
                .values
                .write()
                .expect("the storage lock is never poisoned");
            for _ in 0..count {
                let key = r.chunk()?.to_vec();
                let value = r.chunk()?.to_vec();
                values.insert(key, value);
            }
        }
        if !r.0.is_empty() {
            return Err(Error::BadState);
        }
        let signer = SignatureKeyPair::read(
            provider.storage(),
            &public,
            CIPHERSUITE.signature_algorithm(),
        )
        .ok_or(Error::BadState)?;
        Device::with(provider, signer, &identity)
    }

    fn identity(&self) -> &[u8] {
        self.credential.credential.serialized_content()
    }
}

/// Appends a length-prefixed chunk.
fn put(out: &mut Vec<u8>, chunk: &[u8]) {
    out.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk);
}

/// Reads [`Device::save`]'s bytes.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(Error::BadState)?;
        self.0 = rest;
        Ok(head)
    }

    fn chunk(&mut self) -> Result<&'a [u8], Error> {
        let len = u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| Error::BadState)?);
        self.take(len as usize)
    }
}
