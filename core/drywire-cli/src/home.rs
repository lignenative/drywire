//! What this device keeps between runs, in one directory (`--home`, or
//! `DRYWIRE_HOME`, or `~/.drywire`, `%APPDATA%\drywire` on Windows):
//!
//! - `identity.json`: the device's Ed25519 key, made on first use. The
//!   apps keep it in the system's secure storage (spec section 8); the CLI
//!   keeps it in a file only its owner can read.
//! - `spaces.json`: for each space joined, the instance, the space's id
//!   and name, and the space key from the invitation link, which the
//!   server never sees. Same permissions.
//!
//! Both hold secrets: whoever reads them can speak as this device, and
//! hear its spaces.

use std::path::{Path, PathBuf};

use drywire_control::{Hex, Id, from_hex, to_hex};
use drywire_proto::crypto::SigningKey;
use serde::{Deserialize, Serialize};

/// The directory and what is in it.
pub struct Home {
    dir: PathBuf,
}

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
    /// A space joined or created now.
    pub fn new(instance: &str, space_id: Id, name: &str, space_key: &[u8; 32]) -> KnownSpace {
        KnownSpace {
            instance: instance.to_string(),
            space_id,
            name: printable(name),
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

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    seed: String,
}

#[derive(Serialize, Deserialize, Default)]
struct SpacesFile {
    spaces: Vec<KnownSpace>,
}

impl Home {
    /// The home directory: `dir` if given, else `DRYWIRE_HOME`, else the
    /// user's default. Created, private, if it does not exist.
    pub fn open(dir: Option<&Path>) -> Result<Home, String> {
        let dir = match dir {
            Some(d) => d.to_path_buf(),
            None => match std::env::var_os("DRYWIRE_HOME") {
                Some(d) => PathBuf::from(d),
                None => default_dir()?,
            },
        };
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Home { dir })
    }

    /// The device's identity, made and saved on first use.
    pub fn identity(&self) -> Result<SigningKey, String> {
        let path = self.dir.join("identity.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let file: IdentityFile =
                    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                let seed = Hex::<32>::parse(&file.seed)
                    .ok_or_else(|| format!("{}: the seed is damaged", path.display()))?;
                Ok(SigningKey::from_bytes(&seed.0))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut seed = [0u8; 32];
                getrandom::fill(&mut seed).map_err(|e| format!("random bytes: {e}"))?;
                let file = IdentityFile {
                    seed: to_hex(&seed),
                };
                write_private(&path, &json(&file))?;
                Ok(SigningKey::from_bytes(&seed))
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Forgets the identity, after it was deleted from its instance.
    pub fn forget_identity(&self) -> Result<(), String> {
        let path = self.dir.join("identity.json");
        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Every space this device knows.
    pub fn spaces(&self) -> Result<Vec<KnownSpace>, String> {
        let path = self.dir.join("spaces.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<SpacesFile>(&text)
                .map(|f| f.spaces)
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Remembers a space, replacing what was known of it.
    pub fn remember(&self, space: KnownSpace) -> Result<(), String> {
        let mut spaces = self.spaces()?;
        spaces.retain(|s| !(s.instance == space.instance && s.space_id == space.space_id));
        spaces.push(space);
        self.save(spaces)
    }

    /// Forgets the spaces of an instance (all of them if `space` is None).
    pub fn forget(&self, instance: &str, space: Option<&Id>) -> Result<(), String> {
        let mut spaces = self.spaces()?;
        spaces.retain(|s| !(s.instance == instance && space.is_none_or(|id| *id == s.space_id)));
        self.save(spaces)
    }

    fn save(&self, spaces: Vec<KnownSpace>) -> Result<(), String> {
        write_private(&self.dir.join("spaces.json"), &json(&SpacesFile { spaces }))
    }

    /// The space `which` names, by name (any case) or id; without a name,
    /// the only space known.
    pub fn find(&self, which: Option<&str>) -> Result<KnownSpace, String> {
        let spaces = self.spaces()?;
        let matches: Vec<&KnownSpace> = match which {
            None => spaces.iter().collect(),
            Some(w) => spaces
                .iter()
                .filter(|s| s.name.eq_ignore_ascii_case(w) || s.space_id.to_hex() == w)
                .collect(),
        };
        match (matches.as_slice(), which) {
            ([one], _) => Ok((*one).clone()),
            ([], None) => Err(
                "this device knows no space yet: create one (create-space) or join one with \
                 an invitation link (join --link)"
                    .into(),
            ),
            ([], Some(w)) => Err(format!("no space named {w:?}: see `drywire spaces`")),
            (_, _) => Err("several spaces match: name one with --space (its name or id)".into()),
        }
    }
}

/// A name from a server, safe to print: control characters, which could
/// move the cursor or rewrite the terminal, become `?`. An honest server
/// never sends them (protocol section 13.4); another one might.
pub fn printable(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serializable") + "\n"
}

fn default_dir() -> Result<PathBuf, String> {
    let (var, sub) = if cfg!(windows) {
        ("APPDATA", "drywire")
    } else {
        ("HOME", ".drywire")
    };
    std::env::var_os(var)
        .map(|base| PathBuf::from(base).join(sub))
        .ok_or_else(|| "no home directory: set DRYWIRE_HOME or pass --home".into())
}

/// Writes a file readable by its owner only, where the system has owners,
/// through a temporary file renamed over the old one: a crash leaves the
/// old file or the new one, never half of one. The temporary file has a
/// random name and must not exist yet, so a link planted in a shared
/// directory cannot redirect the write.
pub fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|e| format!("random bytes: {e}"))?;
    let tmp = path.with_extension(format!("{}.tmp", to_hex(&random)));
    let mut options = std::fs::OpenOptions::new();
    // create_new fails on any existing path, a symbolic link included.
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

    fn temp_home(name: &str) -> (PathBuf, Home) {
        let dir = std::env::temp_dir().join(format!("drywire-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = Home::open(Some(&dir)).unwrap();
        (dir, home)
    }

    #[test]
    fn the_identity_is_made_once_and_kept_private() {
        let (dir, home) = temp_home("identity");
        let first = home.identity().unwrap();
        assert_eq!(home.identity().unwrap().to_bytes(), first.to_bytes());
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
    fn spaces_are_found_by_name_or_id() {
        let (dir, home) = temp_home("spaces");
        assert!(home.find(None).is_err());
        let a = KnownSpace::new("https://a.example", Hex([1; 16]), "Team", &[7; 32]);
        home.remember(a.clone()).unwrap();
        assert_eq!(home.find(None).unwrap().space_key().unwrap(), [7; 32]);
        home.remember(KnownSpace::new(
            "https://a.example",
            Hex([2; 16]),
            "Scrims",
            &[8; 32],
        ))
        .unwrap();
        assert!(home.find(None).is_err(), "two spaces: which one?");
        assert_eq!(home.find(Some("team")).unwrap().space_id, Hex([1; 16]));
        assert_eq!(
            home.find(Some(&Hex::<16>([2; 16]).to_hex())).unwrap().name,
            "Scrims"
        );
        // Remembering again replaces, never duplicates.
        home.remember(a).unwrap();
        assert_eq!(home.spaces().unwrap().len(), 2);
        home.forget("https://a.example", Some(&Hex([1; 16])))
            .unwrap();
        assert_eq!(home.spaces().unwrap().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
