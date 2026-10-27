//! The client side of Drywire's control plane (protocol section 13): log
//! in with the device's identity, manage spaces and invitations, join a
//! room, and follow who comes and goes.
//!
//! It is plain blocking HTTPS, through `ureq` with `rustls` and the
//! operating system's certificate store (ADR 0022). Nothing here touches
//! audio: a [`JoinAnswer`] holds what the call needs, and the caller hands
//! it to `drywire-core`.
//!
//! ```no_run
//! # use drywire_control::Client;
//! # use drywire_proto::crypto::SigningKey;
//! # fn main() -> Result<(), drywire_control::Error> {
//! let identity = SigningKey::from_bytes(&[7; 32]);
//! let client = Client::new("https://voice.example.org", identity)?;
//! for membership in client.spaces()? {
//!     println!("{} ({})", membership.space.name, membership.role);
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod events;
mod types;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use drywire_proto::crypto::SigningKey;
use drywire_proto::invite::{CODE_SIZE, Invitation};
use drywire_proto::login::{self, CHALLENGE_SIZE, canonical_origin};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub use events::Events;
pub use types::*;

/// Why a request failed.
#[derive(Debug)]
pub enum Error {
    /// The server answered with an error status and this message, which is
    /// safe to show.
    Refused {
        /// The HTTP status.
        status: u16,
        /// The server's message.
        message: String,
    },
    /// The server could not be reached, or the connection broke.
    Network(String),
    /// The server's answer was not what the protocol says.
    Protocol(String),
    /// The instance address cannot be used.
    BadInstance(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Refused { status, message } => write!(f, "{message} (HTTP {status})"),
            Error::Network(e) => write!(f, "cannot reach the server: {e}"),
            Error::Protocol(e) => write!(f, "unexpected answer from the server: {e}"),
            Error::BadInstance(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Error {
        Error::Network(e.to_string())
    }
}

impl Error {
    /// The HTTP status, if the server refused.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Refused { status, .. } => Some(*status),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
    Patch,
    Delete,
}

/// A logged-in connection to one instance. It logs in on first use, and
/// again whenever its token expires.
pub struct Client {
    agent: Agent,
    /// For event streams: no overall time limit, since they last.
    stream_agent: Agent,
    instance: String,
    identity: SigningKey,
    token: Mutex<Option<String>>,
}

/// Whether a host is this machine, where plain HTTP is allowed.
fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn agent(timeout: Option<Duration>) -> Agent {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(timeout)
        .timeout_connect(Some(Duration::from_secs(10)))
        .tls_config(tls)
        .user_agent(concat!("drywire/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

impl Client {
    /// A client for the instance at `instance` (`https://voice.example.org`),
    /// which signs in with `identity`. Plain `http://` is refused unless the
    /// instance is on this machine: tokens and session secrets cross this
    /// connection.
    pub fn new(instance: &str, identity: SigningKey) -> Result<Client, Error> {
        let instance = canonical_origin(instance).map_err(|e| Error::BadInstance(e.to_string()))?;
        if let Some(host) = instance.strip_prefix("http://") {
            let host = host
                .rsplit_once(':')
                .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
                .map_or(host, |(h, _)| h);
            if !is_loopback(host) {
                return Err(Error::BadInstance(format!(
                    "{instance}: plain http is only for a server on this machine; use https://"
                )));
            }
        }
        Ok(Client {
            agent: agent(Some(Duration::from_secs(30))),
            stream_agent: agent(None),
            instance,
            identity,
            token: Mutex::new(None),
        })
    }

    /// The instance's canonical origin.
    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// The identity's public key, in hex.
    pub fn public_key(&self) -> PublicKey {
        Hex(self.identity.verifying_key().to_bytes())
    }

    /// Logs in (protocol section 13.2): asks for a challenge, signs it with
    /// the instance's origin, and keeps the token.
    pub fn login(&self) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct Challenge {
            challenge: Hex<CHALLENGE_SIZE>,
        }
        #[derive(Deserialize)]
        struct Token {
            token: String,
        }
        let c: Challenge = self.send(Method::Post, "/v1/auth/challenge", None, None)?;
        let signature = login::sign_login(&self.identity, &c.challenge.0, &self.instance);
        let body = json!({
            "public_key": self.public_key(),
            "challenge": c.challenge,
            "signature": to_hex(&signature.to_bytes()),
        });
        let t: Token = self.send(Method::Post, "/v1/auth/login", Some(&body), None)?;
        *self.token.lock().expect("not poisoned") = Some(t.token);
        Ok(())
    }

    fn token(&self) -> Result<String, Error> {
        if let Some(t) = self.token.lock().expect("not poisoned").clone() {
            return Ok(t);
        }
        self.login()?;
        Ok(self
            .token
            .lock()
            .expect("not poisoned")
            .clone()
            .unwrap_or_default())
    }

    /// One request with the token, logging in again once if it expired.
    fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T, Error> {
        let token = self.token()?;
        match self.send(method, path, body, Some(&token)) {
            Err(e) if e.status() == Some(401) => {
                self.login()?;
                let token = self.token()?;
                self.send(method, path, body, Some(&token))
            }
            other => other,
        }
    }

    fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        token: Option<&str>,
    ) -> Result<T, Error> {
        let url = format!("{}{path}", self.instance);
        let auth = token.map(|t| format!("Bearer {t}"));
        macro_rules! with_auth {
            ($req:expr) => {{
                let req = $req;
                match &auth {
                    Some(a) => req.header("Authorization", a),
                    None => req,
                }
            }};
        }
        let response = match (method, body) {
            (Method::Get, _) => with_auth!(self.agent.get(&url)).call(),
            (Method::Delete, _) => with_auth!(self.agent.delete(&url)).call(),
            (Method::Post, Some(b)) => with_auth!(self.agent.post(&url)).send_json(b),
            (Method::Post, None) => with_auth!(self.agent.post(&url)).send_empty(),
            (Method::Patch, b) => {
                with_auth!(self.agent.patch(&url)).send_json(b.unwrap_or(&Value::Null))
            }
        }?;
        let status = response.status().as_u16();
        let mut body = response.into_body();
        if status >= 400 {
            let message = body
                .read_json::<Value>()
                .ok()
                .and_then(|v| v.get("error")?.as_str().map(String::from))
                .unwrap_or_else(|| "the server refused".into());
            return Err(Error::Refused { status, message });
        }
        if status == 204 {
            return serde_json::from_value(Value::Null).map_err(|e| Error::Protocol(e.to_string()));
        }
        body.read_json::<T>()
            .map_err(|e| Error::Protocol(e.to_string()))
    }

    /// The caller's identity, if it has one on this instance.
    pub fn me(&self) -> Result<Option<Identity>, Error> {
        match self.call(Method::Get, "/v1/me", None) {
            Ok(id) => Ok(Some(id)),
            Err(e) if e.status() == Some(404) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Changes the display name.
    pub fn rename(&self, name: &str) -> Result<Identity, Error> {
        self.call(Method::Patch, "/v1/me", Some(&json!({ "name": name })))
    }

    /// Everything the server stores about this identity, as JSON.
    pub fn export(&self) -> Result<Value, Error> {
        self.call(Method::Get, "/v1/me/export", None)
    }

    /// Deletes this identity from the instance, with its memberships.
    pub fn delete_identity(&self) -> Result<(), Error> {
        self.call(Method::Delete, "/v1/me", None)
    }

    /// The caller's spaces.
    pub fn spaces(&self) -> Result<Vec<Membership>, Error> {
        self.call(Method::Get, "/v1/spaces", None)
    }

    /// Creates a space with one room. `display_name` names the caller if
    /// this is its first space on the instance.
    pub fn create_space(
        &self,
        name: &str,
        room: &str,
        display_name: &str,
    ) -> Result<SpaceView, Error> {
        let body = json!({ "name": name, "room": room, "display_name": display_name });
        self.call(Method::Post, "/v1/spaces", Some(&body))
    }

    /// A space, with its rooms and members.
    pub fn space(&self, space: &Id) -> Result<SpaceView, Error> {
        self.call(Method::Get, &format!("/v1/spaces/{space}"), None)
    }

    /// Adds a room to a space (admins).
    pub fn create_room(&self, space: &Id, name: &str) -> Result<Room, Error> {
        let body = json!({ "name": name });
        self.call(
            Method::Post,
            &format!("/v1/spaces/{space}/rooms"),
            Some(&body),
        )
    }

    /// Leaves a space.
    pub fn leave_space(&self, space: &Id) -> Result<(), Error> {
        let path = format!("/v1/spaces/{space}/members/{}", self.public_key());
        self.call(Method::Delete, &path, None)
    }

    /// Creates an invitation code (admins) and builds the link with the
    /// space key, which never leaves this machine except in the link.
    /// `expires_in` is in seconds; `None` takes the server's defaults.
    pub fn invite(
        &self,
        space: &Id,
        space_key: &[u8; 32],
        expires_in: Option<u64>,
        max_uses: Option<u32>,
    ) -> Result<(Invitation, InvitationInfo), Error> {
        let body = json!({
            "expires_in": expires_in.unwrap_or(0),
            "max_uses": max_uses.unwrap_or(0),
        });
        let created: NewInvitation = self.call(
            Method::Post,
            &format!("/v1/spaces/{space}/invitations"),
            Some(&body),
        )?;
        // The link carries the origin the client signs, so the next member
        // reaches the instance the way this one does.
        let invitation = Invitation {
            instance: self.instance.clone(),
            code: created.code.0,
            space_key: *space_key,
        };
        Ok((invitation, created.invitation))
    }

    /// Redeems an invitation's code: the caller becomes a member, with this
    /// display name if it is new on the instance.
    pub fn redeem(&self, code: &[u8; CODE_SIZE], display_name: &str) -> Result<SpaceView, Error> {
        let body = json!({ "code": to_hex(code), "display_name": display_name });
        self.call(Method::Post, "/v1/invitations/redeem", Some(&body))
    }

    /// Joins a room: a new UDP session, and who is already there.
    pub fn join(&self, room: &Id) -> Result<JoinAnswer, Error> {
        self.call(Method::Post, &format!("/v1/rooms/{room}/join"), None)
    }

    /// Ends this identity's session.
    pub fn leave(&self) -> Result<(), Error> {
        self.call(Method::Post, "/v1/leave", None)
    }

    /// Opens a space's event stream. It blocks the thread that reads it.
    pub fn events(&self, space: &Id) -> Result<Events, Error> {
        let url = format!("{}/v1/spaces/{space}/events", self.instance);
        let open = |token: &str| {
            self.stream_agent
                .get(&url)
                .header("Authorization", &format!("Bearer {token}"))
                .header("Accept", "text/event-stream")
                .call()
        };
        let mut response = open(&self.token()?)?;
        if response.status().as_u16() == 401 {
            self.login()?;
            response = open(&self.token()?)?;
        }
        let status = response.status().as_u16();
        if status != 200 {
            let message = response
                .into_body()
                .read_json::<Value>()
                .ok()
                .and_then(|v| v.get("error")?.as_str().map(String::from))
                .unwrap_or_else(|| "the server refused".into());
            return Err(Error::Refused { status, message });
        }
        Ok(Events::new(Box::new(response.into_body().into_reader())))
    }
}
