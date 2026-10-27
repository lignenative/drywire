//! Logging in to the control plane (section 13.2): the client signs a
//! challenge from the server with its identity key, and gets a short-lived
//! token in return.

use ed25519_dalek::{Signature, Signer};

use crate::crypto::SigningKey;

/// Size of a login challenge.
pub const CHALLENGE_SIZE: usize = 32;

const LOGIN_INFO: &[u8] = b"drywire/login/v1";

/// The message a client signs to log in:
/// `"drywire/login/v1" ‖ challenge (32) ‖ audience`.
///
/// The audience is the instance's canonical origin ([`canonical_origin`]),
/// as the client reached it: a malicious instance that passes on another
/// instance's challenge gets a signature the other instance refuses.
pub fn login_message(challenge: &[u8; CHALLENGE_SIZE], audience: &str) -> Vec<u8> {
    [LOGIN_INFO, challenge, audience.as_bytes()].concat()
}

/// Signs the login message with the identity key.
pub fn sign_login(
    identity: &SigningKey,
    challenge: &[u8; CHALLENGE_SIZE],
    audience: &str,
) -> Signature {
    identity.sign(&login_message(challenge, audience))
}

/// Why an instance address was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginError(pub(crate) String);

impl std::fmt::Display for OriginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OriginError {}

/// The canonical form of an instance address, which logins sign and
/// invitation links hold: lowercase scheme and host, no default port, no
/// path, query, fragment or user name. `https://Voice.Example.org:443/`
/// becomes `https://voice.example.org`.
pub fn canonical_origin(address: &str) -> Result<String, OriginError> {
    let bad = |why: &str| OriginError(format!("instance address {address:?}: {why}"));
    let (scheme, rest) = address
        .split_once("://")
        .ok_or_else(|| bad("must start with https:// or http://"))?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => "443",
        "http" => "80",
        _ => return Err(bad("must start with https:// or http://")),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if !(tail.is_empty() || tail == "/") {
        return Err(bad(
            "must be a scheme, a host and an optional port, nothing else",
        ));
    }
    if authority.contains('@') {
        return Err(bad("must not hold a user name"));
    }
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (host, after) = inner
            .split_once(']')
            .ok_or_else(|| bad("has an unclosed ["))?;
        let port = match after {
            "" => None,
            p => Some(p.strip_prefix(':').ok_or_else(|| bad("has a bad port"))?),
        };
        (format!("[{}]", host.to_ascii_lowercase()), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_ascii_lowercase(), Some(port)),
            None => (authority.to_ascii_lowercase(), None),
        }
    };
    if host.is_empty() || host == "[]" {
        return Err(bad("has no host"));
    }
    match port {
        None => Ok(format!("{scheme}://{host}")),
        Some(p) if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) => {
            Err(bad("has a bad port"))
        }
        Some(p) => {
            let n: u16 = p.parse().map_err(|_| bad("has a bad port"))?;
            if n == 0 {
                Err(bad("has a bad port"))
            } else if p == default_port {
                Ok(format!("{scheme}://{host}"))
            } else {
                Ok(format!("{scheme}://{host}:{n}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_canonical() {
        for (input, want) in [
            ("https://Voice.Example.org", "https://voice.example.org"),
            (
                "HTTPS://voice.example.org:443/",
                "https://voice.example.org",
            ),
            (
                "https://voice.example.org:8443",
                "https://voice.example.org:8443",
            ),
            ("http://127.0.0.1:10050", "http://127.0.0.1:10050"),
            ("http://localhost:80", "http://localhost"),
            ("http://[::1]:10050", "http://[::1]:10050"),
            ("https://[2001:DB8::1]", "https://[2001:db8::1]"),
        ] {
            assert_eq!(canonical_origin(input).as_deref(), Ok(want), "{input}");
        }
        for input in [
            "voice.example.org",
            "ftp://voice.example.org",
            "https://a@voice.example.org",
            "https://voice.example.org/path",
            "https://voice.example.org/?q",
            "https://voice.example.org#f",
            "https://",
            "https://host:",
            "https://host:0",
            "https://host:99999",
            "https://host:+1",
        ] {
            assert!(canonical_origin(input).is_err(), "{input} accepted");
        }
    }
}
