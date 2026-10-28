//! The app without its window: what each command does. The Tauri commands
//! in `commands.rs` only call these methods, so the integration tests
//! drive the whole app, against a real server, without a window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use drywire_control::{Client, Id};
use drywire_proto::invite::Invitation;
use serde::Serialize;

use crate::error::AppError;
use crate::session::{CallView, DeviceFactory, JoinRequest, Session, printable};
use crate::store::{KnownSpace, Store};

/// Makes the factory that opens the sound card for each call.
pub type Devices = Arc<dyn Fn() -> DeviceFactory + Send + Sync>;

/// A room, for display.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    /// Its id, in hex.
    pub id: String,
    /// Its name.
    pub name: String,
}

/// A space this device knows, with its rooms as the server lists them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceView {
    /// The instance's origin.
    pub instance: String,
    /// Its id, in hex.
    pub id: String,
    /// Its name.
    pub name: String,
    /// This device's role: `owner`, `admin` or `member`; empty when the
    /// server could not be reached.
    pub role: String,
    /// Its rooms, by name.
    pub rooms: Vec<RoomView>,
    /// Why the rooms could not be listed, if they could not.
    pub error: Option<AppError>,
}

/// The app's state, shared by every command.
pub struct Desktop {
    store: Store,
    devices: Devices,
    /// Held while joining or leaving, so two of them never overlap; the
    /// session itself is behind a lock held only for an instant, so
    /// polling the call never waits for a join.
    switching: Mutex<()>,
    session: Mutex<Option<Session>>,
    /// One logged-in client per instance, kept for the app's lifetime:
    /// logging in for every command would spend the login budget the
    /// server gives each network.
    clients: Mutex<HashMap<String, Arc<Client>>>,
}

impl Desktop {
    /// The app on `store`, opening sound cards with `devices`.
    pub fn new(store: Store, devices: Devices) -> Desktop {
        Desktop {
            store,
            devices,
            switching: Mutex::new(()),
            session: Mutex::new(None),
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// The device identity's public key, in hex: what an operator lists to
    /// let this device create spaces.
    pub fn public_key(&self) -> Result<String, AppError> {
        let identity = self.store.identity().map_err(AppError::storage)?;
        Ok(drywire_control::to_hex(identity.verifying_key().as_bytes()))
    }

    /// Redeems an invitation link and remembers its space.
    pub fn accept(&self, link: &str, display_name: &str) -> Result<SpaceView, AppError> {
        let invitation =
            Invitation::parse(link).map_err(|e| AppError::new("invalid_link", e.to_string()))?;
        let client = self.client(&invitation.instance)?;
        let view = client.redeem(&invitation.code, display_name.trim())?;
        self.store
            .remember(KnownSpace::new(
                client.instance(),
                view.space.id,
                &view.space.name,
                &invitation.space_key,
            ))
            .map_err(AppError::storage)?;
        Ok(space_view(client.instance(), &view))
    }

    /// Every space this device knows, with its rooms. A server that cannot
    /// be reached does not hide the others: its spaces carry the error.
    pub fn spaces(&self) -> Result<Vec<SpaceView>, AppError> {
        let spaces = self.store.spaces().map_err(AppError::storage)?;
        Ok(spaces
            .iter()
            .map(|known| {
                let listed = self
                    .client(&known.instance)
                    .and_then(|client| Ok(client.space(&known.space_id)?));
                match listed {
                    Ok(view) => {
                        // Keep the name the server knows now.
                        if view.space.name != known.name {
                            let mut renamed = known.clone();
                            renamed.name = view.space.name.clone();
                            let _ = self.store.remember(renamed);
                        }
                        space_view(&known.instance, &view)
                    }
                    Err(e) => SpaceView {
                        instance: known.instance.clone(),
                        id: known.space_id.to_hex(),
                        name: printable(&known.name),
                        role: String::new(),
                        rooms: Vec::new(),
                        error: Some(e),
                    },
                }
            })
            .collect())
    }

    /// Joins a room of a known space, leaving the current call first.
    pub fn join(
        &self,
        instance: &str,
        space_id: &str,
        room_id: &str,
    ) -> Result<CallView, AppError> {
        let _switching = self.switching.lock().expect("not poisoned");
        let space_id = parse_id(space_id)?;
        let room_id = parse_id(room_id)?;
        let known = self
            .store
            .space(instance, &space_id)
            .map_err(|e| AppError::new("not_found", e))?;
        let request = JoinRequest {
            client: self.client(&known.instance)?,
            identity: self.store.identity().map_err(AppError::storage)?,
            space_id,
            space_name: known.name.clone(),
            space_key: known.space_key().map_err(AppError::storage)?,
            room_id,
        };
        // The next join replaces this device's session on the server, so
        // there is no need to tell it we left.
        if let Some(previous) = self.take_session() {
            previous.end(false);
        }
        let session = Session::start(request, (self.devices)())?;
        let view = session.view();
        *self.session.lock().expect("not poisoned") = Some(session);
        Ok(view)
    }

    /// Leaves the call, if any.
    pub fn leave(&self) {
        let _switching = self.switching.lock().expect("not poisoned");
        if let Some(session) = self.take_session() {
            session.end(true);
        }
    }

    /// What the interface shows about the call, if one runs.
    pub fn call(&self) -> Option<CallView> {
        self.session
            .lock()
            .expect("not poisoned")
            .as_ref()
            .map(Session::view)
    }

    /// The client for `instance`, made on first use.
    fn client(&self, instance: &str) -> Result<Arc<Client>, AppError> {
        let mut clients = self.clients.lock().expect("not poisoned");
        if let Some(client) = clients.get(instance) {
            return Ok(Arc::clone(client));
        }
        let identity = self.store.identity().map_err(AppError::storage)?;
        let client = Arc::new(Client::new(instance, identity)?);
        clients.insert(instance.to_string(), Arc::clone(&client));
        Ok(client)
    }

    fn take_session(&self) -> Option<Session> {
        self.session.lock().expect("not poisoned").take()
    }
}

fn parse_id(text: &str) -> Result<Id, AppError> {
    Id::parse(text).ok_or_else(|| AppError::new("not_found", format!("{text:?} is not an id")))
}

fn space_view(instance: &str, view: &drywire_control::SpaceView) -> SpaceView {
    SpaceView {
        instance: instance.to_string(),
        id: view.space.id.to_hex(),
        name: printable(&view.space.name),
        role: printable(&view.role),
        rooms: view
            .rooms
            .iter()
            .map(|r| RoomView {
                id: r.id.to_hex(),
                name: printable(&r.name),
            })
            .collect(),
        error: None,
    }
}
