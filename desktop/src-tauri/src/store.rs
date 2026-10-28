//! What the app keeps between runs, in its data directory
//! (`%APPDATA%\com.lignenative.drywire` on Windows, or
//! `DRYWIRE_DESKTOP_HOME`):
//!
//! - `identity.json`: the device's Ed25519 seed, made on first use.
//! - `spaces.json`: for each space joined, the instance, the space's id and
//!   name, and the space key from the invitation link, which the server
//!   never sees.
//!
//! Both hold secrets, sealed by the [`vault`](crate::vault): whoever opens
//! them can speak as this device and hear its spaces. The desktop app has
//! an identity of its own, apart from the command-line client's, whose
//! files are not protected.
//!
//! - `settings.json`: the person's preferences ([`Settings`]), in plain
//!   JSON: nothing in it is secret.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use drywire_control::{Hex, Id, from_hex, to_hex};
use drywire_proto::crypto::SigningKey;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::vault;

/// A space this device can enter.
#[derive(Clone, Serialize, Deserialize)]
pub struct KnownSpace {
    /// The instance's canonical origin.
    pub instance: String,
    /// The space's id on that instance.
    pub space_id: Id,
    /// Its name, as last seen.
    pub name: String,
    /// The secret every member holds, in hex.
    space_key: String,
}

impl KnownSpace {
    /// A space joined now.
    pub fn new(instance: &str, space_id: Id, name: &str, space_key: &[u8; 32]) -> KnownSpace {
        KnownSpace {
            instance: instance.to_string(),
            space_id,
            name: name.to_string(),
            space_key: to_hex(space_key),
        }
    }

    /// The space key.
    pub fn space_key(&self) -> Result<[u8; 32], String> {
        from_hex(&self.space_key)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| format!("the key of space {} is damaged", self.name))
    }
}

/// The person's preferences. Fields added later take their default when
/// an older file lacks them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// The level, in dBFS, at which the voice gate opens; it closes 6 dB
    /// lower.
    pub gate_open_dbfs: f32,
    /// The volume of each person other than 1 (100 %), by the hex of their
    /// public key: it follows them from room to room.
    pub volumes: BTreeMap<String, f32>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            gate_open_dbfs: drywire_core::gate::GateConfig::default().open_dbfs,
            volumes: BTreeMap::new(),
        }
    }
}

/// A file's contents, sealed: the protection's name and the sealed bytes.
#[derive(Serialize, Deserialize)]
struct Sealed {
    protection: String,
    data: String,
}

/// The data directory.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The store in `dir`, created readable by its owner only if it does
    /// not exist.
    pub fn open(dir: &Path) -> Result<Store, String> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Store {
            dir: dir.to_path_buf(),
        })
    }

    /// The device's identity, made and saved on first use.
    pub fn identity(&self) -> Result<SigningKey, String> {
        let path = self.dir.join("identity.json");
        if let Some(seed) = read_sealed::<String>(&path)? {
            let seed = Hex::<32>::parse(&seed)
                .ok_or_else(|| format!("{}: the seed is damaged", path.display()))?;
            return Ok(SigningKey::from_bytes(&seed.0));
        }
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| format!("random bytes: {e}"))?;
        write_sealed(&path, &to_hex(&seed))?;
        Ok(SigningKey::from_bytes(&seed))
    }

    /// Every space this device knows.
    pub fn spaces(&self) -> Result<Vec<KnownSpace>, String> {
        Ok(read_sealed(&self.dir.join("spaces.json"))?.unwrap_or_default())
    }

    /// The space with this id on this instance.
    pub fn space(&self, instance: &str, space_id: &Id) -> Result<KnownSpace, String> {
        self.spaces()?
            .into_iter()
            .find(|s| s.instance == instance && s.space_id == *space_id)
            .ok_or_else(|| "this device does not know that space".to_string())
    }

    /// The preferences; the defaults if none were saved, or if the file is
    /// damaged: a preference is never worth refusing to start.
    pub fn settings(&self) -> Settings {
        std::fs::read_to_string(self.dir.join("settings.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Saves the preferences.
    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        let text = serde_json::to_string_pretty(settings).expect("serializable") + "\n";
        write_private(&self.dir.join("settings.json"), &text)
    }

    /// Remembers a space, replacing what was known of it.
    pub fn remember(&self, space: KnownSpace) -> Result<(), String> {
        let mut spaces = self.spaces()?;
        spaces.retain(|s| !(s.instance == space.instance && s.space_id == space.space_id));
        spaces.push(space);
        write_sealed(&self.dir.join("spaces.json"), &spaces)
    }
}

/// Reads and opens a sealed file; `None` if it does not exist.
fn read_sealed<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err(&e)),
    };
    let sealed: Sealed = serde_json::from_str(&text).map_err(|e| err(&e))?;
    let data = from_hex(&sealed.data).ok_or_else(|| err(&"damaged"))?;
    let plain = vault::open(&sealed.protection, &data).map_err(|e| err(&e))?;
    serde_json::from_slice(&plain)
        .map(Some)
        .map_err(|e| err(&e))
}

/// Seals `value` and writes it.
fn write_sealed<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let plain = serde_json::to_vec(value).expect("serializable");
    let sealed = Sealed {
        protection: vault::PROTECTION.to_string(),
        data: to_hex(&vault::seal(&plain)?),
    };
    let text = serde_json::to_string_pretty(&sealed).expect("serializable") + "\n";
    write_private(path, &text)
}

/// Writes a file readable by its owner only, where the system has owners,
/// through a temporary file renamed over the old one: a crash leaves the
/// old file or the new one, never half of one. The temporary file has a
/// random name and must not exist yet, so a link planted in the directory
/// cannot redirect the write.
pub fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|e| format!("random bytes: {e}"))?;
    let tmp = path.with_extension(format!("{}.tmp", to_hex(&random)));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(err)?;
    std::io::Write::write_all(&mut file, contents.as_bytes()).map_err(err)?;
    file.sync_all().map_err(err)?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (PathBuf, Store) {
        let dir =
            std::env::temp_dir().join(format!("drywire-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        (dir, store)
    }

    #[test]
    fn the_identity_is_made_once_and_sealed() {
        let (dir, store) = temp_store("identity");
        let first = store.identity().unwrap();
        assert_eq!(store.identity().unwrap().to_bytes(), first.to_bytes());
        let text = std::fs::read_to_string(dir.join("identity.json")).unwrap();
        assert!(text.contains(vault::PROTECTION), "{text}");
        if cfg!(windows) {
            let seed = to_hex(&first.to_bytes());
            assert!(!text.contains(&seed), "DPAPI must hide the seed");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir.join("identity.json")), 0o600);
            assert_eq!(mode(&dir), 0o700);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_survive_and_default_when_missing_or_damaged() {
        let (dir, store) = temp_store("settings");
        assert_eq!(store.settings(), Settings::default());
        let settings = Settings {
            gate_open_dbfs: -52.0,
            volumes: [("ab".repeat(32), 1.5)].into(),
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.settings(), settings);
        std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
        assert_eq!(store.settings(), Settings::default());
        // A file from an older version, without the newer fields.
        std::fs::write(dir.join("settings.json"), r#"{"gateOpenDbfs": -45}"#).unwrap();
        assert_eq!(store.settings().gate_open_dbfs, -45.0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn spaces_are_remembered_once() {
        let (dir, store) = temp_store("spaces");
        assert!(store.spaces().unwrap().is_empty());
        let team = KnownSpace::new("https://voice.example.org", Hex([1; 16]), "Team", &[7; 32]);
        store.remember(team.clone()).unwrap();
        store.remember(team).unwrap();
        let spaces = store.spaces().unwrap();
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].space_key().unwrap(), [7; 32]);
        let found = store
            .space("https://voice.example.org", &Hex([1; 16]))
            .unwrap();
        assert_eq!(found.name, "Team");
        assert!(
            store
                .space("https://other.example.org", &Hex([1; 16]))
                .is_err()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
