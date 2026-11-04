//! Accounts by e-mail and user name (ADR 0055, protocol section 13.8).

use drywire_proto::login::{self, CHALLENGE_SIZE};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Client, Error, Hex, Id, Identity, Method, PublicKey, SpaceView, refused, to_hex};

/// What signing in by e-mail gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedIn {
    /// The address had no account: one was made, which chooses its user
    /// name next ([`Client::set_username`]).
    pub new_account: bool,
    /// The account's user name, if it has one.
    pub username: Option<String>,
}

/// One device of the account.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AccountDevice {
    /// Its key.
    pub public_key: PublicKey,
    /// Whether it is the device asking.
    pub this_device: bool,
}

/// Someone found by user name.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct User {
    /// Their user name.
    pub username: String,
    /// Their display name.
    pub name: String,
}

/// An invitation waiting for this account.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PendingInvitation {
    /// The space.
    pub space_id: Id,
    /// Its name.
    pub space_name: String,
    /// Who invited, by user name; empty if their account is gone.
    pub invited_by: String,
}

impl Client {
    /// Asks the instance to mail a sign-in code to `email`, in `lang`
    /// ("en" or "fr").
    pub fn request_email_code(&self, email: &str, lang: &str) -> Result<(), Error> {
        let body = json!({ "email": email, "lang": lang });
        let (status, value) = self.request(Method::Post, "/v1/auth/email", Some(&body), None)?;
        if status >= 400 {
            return Err(refused(status, &value));
        }
        Ok(())
    }

    /// Signs this device in with the code mailed to `email`: the device's
    /// key signs a login challenge alongside, and the token is kept.
    pub fn verify_email_code(&self, email: &str, code: &str) -> Result<SignedIn, Error> {
        #[derive(Deserialize)]
        struct Challenge {
            challenge: Hex<CHALLENGE_SIZE>,
        }
        let c: Challenge = self.send(Method::Post, "/v1/auth/challenge", None, None)?;
        let signature = login::sign_login(&self.identity, &c.challenge.0, &self.instance);
        let body = json!({
            "email": email,
            "code": code,
            "public_key": self.public_key(),
            "challenge": c.challenge,
            "signature": to_hex(&signature.to_bytes()),
        });
        let (status, value) =
            self.request(Method::Post, "/v1/auth/email/verify", Some(&body), None)?;
        if status == 401 && value.get("tries_left").is_some() {
            let tries_left = value["tries_left"].as_u64().unwrap_or(0) as u32;
            return Err(Error::WrongCode { tries_left });
        }
        if status >= 400 {
            return Err(refused(status, &value));
        }
        let token = value["token"]
            .as_str()
            .ok_or_else(|| Error::Protocol("no token".into()))?;
        *self.token.lock().expect("not poisoned") = Some(token.to_string());
        Ok(SignedIn {
            new_account: value["new_account"].as_bool().unwrap_or(false),
            username: value["username"]
                .as_str()
                .filter(|u| !u.is_empty())
                .map(String::from),
        })
    }

    /// Chooses the account's user name, and its display name (the user
    /// name when empty).
    pub fn set_username(&self, username: &str, display_name: &str) -> Result<Identity, Error> {
        let body = json!({ "username": username, "display_name": display_name });
        match self.call_value(Method::Put, "/v1/me/username", Some(&body))? {
            (409, value) => Err(Error::UsernameTaken {
                suggestions: serde_json::from_value(value["suggestions"].clone())
                    .unwrap_or_default(),
            }),
            (status, value) if status >= 400 => Err(refused(status, &value)),
            (_, value) => serde_json::from_value(value).map_err(|e| Error::Protocol(e.to_string())),
        }
    }

    /// The account's devices.
    pub fn devices(&self) -> Result<Vec<AccountDevice>, Error> {
        self.call(Method::Get, "/v1/me/devices", None)
    }

    /// Signs one of the account's devices out for good.
    pub fn remove_device(&self, key: &PublicKey) -> Result<(), Error> {
        self.call(Method::Delete, &format!("/v1/me/devices/{key}"), None)
    }

    /// The account with exactly this user name, if any.
    pub fn find_user(&self, username: &str) -> Result<Option<User>, Error> {
        let path = format!("/v1/users/{}", username.trim().to_lowercase());
        match self.call(Method::Get, &path, None) {
            Ok(user) => Ok(Some(user)),
            Err(e) if e.status() == Some(404) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Invites someone into the space by user name.
    pub fn invite_user(&self, space: &Id, username: &str) -> Result<User, Error> {
        let path = format!("/v1/spaces/{space}/invitations/users");
        self.call(Method::Post, &path, Some(&json!({ "username": username })))
    }

    /// The invitations waiting for this account.
    pub fn invitations(&self) -> Result<Vec<PendingInvitation>, Error> {
        self.call(Method::Get, "/v1/me/invitations", None)
    }

    /// Accepts an invitation: the space, as a redeemed code gives it.
    pub fn accept_invitation(&self, space: &Id) -> Result<SpaceView, Error> {
        self.call(Method::Post, &format!("/v1/me/invitations/{space}"), None)
    }

    /// Declines an invitation.
    pub fn decline_invitation(&self, space: &Id) -> Result<(), Error> {
        let _: Value = self.call(Method::Delete, &format!("/v1/me/invitations/{space}"), None)?;
        Ok(())
    }
}
