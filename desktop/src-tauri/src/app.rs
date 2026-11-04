//! The app without its window: what each command does. The Tauri commands
//! in `commands.rs` only call these methods, so the integration tests
//! drive the whole app, against a real server, without a window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use drywire_control::{Client, Hex, Id, VaultContents, VaultKind, VaultSpace};
use drywire_proto::invite::Invitation;
use drywire_proto::vault::VaultCode;
use serde::Serialize;

use crate::error::AppError;
use crate::meter::Meter;
use crate::rooms::{Changed, Rooms};
use crate::session::{
    CallView, DeviceView, JoinRequest, METER_FLOOR_DBFS, Notices, Session, clamp_gate, printable,
};
use crate::store::{KnownSpace, Settings, Store};

/// What the loopback test measured.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopbackView {
    /// The median delay, input plus output, in milliseconds; `None` when
    /// no click was heard.
    pub median_ms: Option<f64>,
    /// Every delay measured, in milliseconds.
    pub delays_ms: Vec<f64>,
    /// How many clicks were played.
    pub clicks: usize,
    /// What the system says of the two streams, to compare.
    pub devices: Vec<DeviceView>,
}

/// What the gate calibration found.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationView {
    /// The background noise's typical level, in dBFS.
    pub noise_dbfs: f32,
    /// Its loudest moments, in dBFS.
    pub peak_dbfs: f32,
    /// The threshold proposed, in dBFS: the interface applies it with
    /// `set_gate` once the person accepts.
    pub gate_dbfs: f32,
    /// Whether the room is too loud for quiet speech to open the gate.
    pub too_loud: bool,
}

/// How many clicks the loopback test waits for.
const LOOPBACK_CLICKS: usize = 9;

/// What the settings panel edits.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// The global mute shortcut, such as `Ctrl+Shift+M`.
    pub mute_shortcut: String,
    /// The presence sounds' volume, 0 (off) to 1.
    pub sound_volume: f32,
    /// Whether the overlay shows during calls.
    pub overlay: bool,
    /// Its corner (see [`crate::store::OVERLAY_CORNERS`]); anything else
    /// keeps the current one.
    pub overlay_corner: String,
}

pub use crate::session::Devices;

/// A room, for display.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    /// Its id, in hex.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Who sits in it, sorted by name; absent when unknown (the space's
    /// event stream is not open, or [`crate::rooms::MAX_PER_INSTANCE`]
    /// spaces of its instance are already followed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occupants: Option<Vec<String>>,
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

/// The account as the settings show it (ADR 0051).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    /// Whether this device knows the account's current recovery code.
    pub recovery_code: bool,
    /// How many spaces the account has: none, and it has no instance to
    /// keep a vault on.
    pub spaces: usize,
    /// Whether the account's key can be put in a vault (always, in the
    /// app; not for an old browser key in the web client).
    pub portable: bool,
}

/// What opening a code brought: the account's name and its spaces.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RestoredView {
    /// The display name the vault holds.
    pub name: String,
    /// The spaces, as [`Desktop::spaces`] lists them.
    pub spaces: Vec<SpaceView>,
}

/// A device code, waiting on an instance.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodeView {
    /// The code, in groups of four.
    pub code: String,
    /// The instance it waits on.
    pub instance: String,
    /// Seconds until it stops working.
    pub expires_in_s: u64,
}

/// The app's state, shared by every command.
pub struct Desktop {
    store: Store,
    devices: Devices,
    notices: Notices,
    /// Held while joining or leaving, so two of them never overlap; the
    /// session itself is behind a lock held only for an instant, so
    /// polling the call never waits for a join.
    switching: Mutex<()>,
    session: Mutex<Option<Session>>,
    /// The microphone's meter out of a call, while the settings show it.
    meter: Mutex<Option<Meter>>,
    /// Who sits in the rooms of the known spaces.
    rooms: Rooms,
    /// The spaces as last listed, without their occupants: what a change
    /// of occupants is shown with, without asking the servers again.
    listed: Mutex<Vec<SpaceView>>,
    /// One logged-in client per instance, kept for the app's lifetime:
    /// logging in for every command would spend the login budget the
    /// server gives each network.
    clients: Mutex<HashMap<String, Arc<Client>>>,
}

impl Desktop {
    /// The app on `store`, opening sound cards with `devices`, telling
    /// `notices` who comes and goes in the room, and `rooms_changed` when
    /// someone sits down in or leaves any room of the known spaces
    /// ([`Desktop::spaces_now`] then has the new names).
    pub fn new(
        store: Store,
        devices: Devices,
        notices: Notices,
        rooms_changed: Changed,
    ) -> Desktop {
        Desktop {
            store,
            devices,
            notices,
            switching: Mutex::new(()),
            session: Mutex::new(None),
            meter: Mutex::new(None),
            rooms: Rooms::new(rooms_changed),
            listed: Mutex::new(Vec::new()),
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// The device identity's public key, in hex: what an operator lists to
    /// let this device create spaces.
    pub fn public_key(&self) -> Result<String, AppError> {
        let identity = self.store.identity().map_err(AppError::storage)?;
        Ok(drywire_control::to_hex(identity.verifying_key().as_bytes()))
    }

    /// The account, for the settings.
    pub fn account(&self) -> Result<AccountView, AppError> {
        Ok(AccountView {
            recovery_code: self
                .store
                .recovery_secret()
                .map_err(AppError::storage)?
                .is_some(),
            spaces: self.store.spaces().map_err(AppError::storage)?.len(),
            portable: true,
        })
    }

    /// The instances of the known spaces, each once, in a stable order.
    fn instances(&self) -> Result<Vec<String>, AppError> {
        let mut all: Vec<String> = self
            .store
            .spaces()
            .map_err(AppError::storage)?
            .into_iter()
            .map(|s| s.instance)
            .collect();
        all.sort();
        all.dedup();
        Ok(all)
    }

    /// What a vault of this account holds, with the name `client`'s
    /// instance knows.
    fn vault_contents(&self, client: &Client) -> Result<VaultContents, AppError> {
        let identity = self.store.identity().map_err(AppError::storage)?;
        let name = client.me()?.map(|id| id.name).unwrap_or_default();
        let spaces = self
            .store
            .spaces()
            .map_err(AppError::storage)?
            .iter()
            .map(|s| {
                Ok(VaultSpace {
                    instance: s.instance.clone(),
                    id: s.space_id,
                    name: s.name.clone(),
                    key: Hex(s.space_key().map_err(AppError::storage)?),
                })
            })
            .collect::<Result<_, AppError>>()?;
        Ok(VaultContents {
            account: Hex(identity.to_bytes()),
            name,
            spaces,
        })
    }

    /// Seals the recovery vault with `code` on every instance of the spaces.
    fn put_recovery(&self, code: &VaultCode) -> Result<(), AppError> {
        let instances = self.instances()?;
        let Some(first) = instances.first() else {
            return Err(AppError::new("no_space", "join a space first"));
        };
        let contents = self.vault_contents(&*self.client(first)?)?;
        for instance in &instances {
            self.client(instance)?.put_recovery_vault(code, &contents)?;
        }
        Ok(())
    }

    /// Seals the recovery vault again after the spaces changed, if this
    /// device knows the code. A failure is not the change's: it is left
    /// for the next change, or a new code.
    fn seal_recovery_again(&self) {
        if let Ok(Some(secret)) = self.store.recovery_secret() {
            let _ = self.put_recovery(&VaultCode::from_secret(secret));
        }
    }

    /// Makes a new recovery code, which replaces the old one on every
    /// instance, and returns it to show once.
    pub fn new_recovery_code(&self) -> Result<String, AppError> {
        let code = drywire_control::new_code();
        self.put_recovery(&code)?;
        self.store
            .set_recovery_secret(Some(&code.secret()))
            .map_err(AppError::storage)?;
        Ok(code.to_text())
    }

    /// Makes a device code, on the first instance of the spaces.
    pub fn new_device_code(&self) -> Result<DeviceCodeView, AppError> {
        let instance = self
            .instances()?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::new("no_space", "join a space first"))?;
        let client = self.client(&instance)?;
        let code = drywire_control::new_code();
        let waiting = client.put_device_vault(&code, &self.vault_contents(&client)?)?;
        Ok(DeviceCodeView {
            code: code.to_text(),
            instance: client.instance().to_string(),
            expires_in_s: waiting.expires_in,
        })
    }

    /// Becomes the account a code opens on `instance`. If this device
    /// already uses another account in some space, it refuses with
    /// `account_exists` unless `replace` says to go on. The call, the
    /// instances' clients and the followed rooms of the old account end.
    pub fn restore_account(
        &self,
        instance: &str,
        code: &str,
        replace: bool,
    ) -> Result<RestoredView, AppError> {
        let (kind, contents) =
            drywire_control::open_vault(instance, code).map_err(|e| match e.status() {
                Some(404) => AppError::new("code_unknown", e.to_string()),
                _ => AppError::from(e),
            })?;
        let account = contents.signing_key();
        let current = self.store.spaces().map_err(AppError::storage)?;
        let identity = self.store.identity().map_err(AppError::storage)?;
        let same = identity.verifying_key() == account.verifying_key();
        if !same && !current.is_empty() && !replace {
            return Err(AppError::new(
                "account_exists",
                format!(
                    "this device already has an account, with {} space(s)",
                    current.len()
                ),
            ));
        }
        let mut spaces: Vec<KnownSpace> = contents
            .spaces
            .iter()
            .map(|s| KnownSpace::new(&s.instance, s.id, &s.name, &s.key.0))
            .collect();
        if same {
            for s in current {
                if !spaces
                    .iter()
                    .any(|v| v.instance == s.instance && v.space_id == s.space_id)
                {
                    spaces.push(s);
                }
            }
        }
        self.leave();
        self.close_meter();
        self.store
            .become_account(&account.to_bytes(), &spaces)
            .map_err(AppError::storage)?;
        if kind == VaultKind::Recovery
            && let Ok(code) = VaultCode::parse(code)
        {
            self.store
                .set_recovery_secret(Some(&code.secret()))
                .map_err(AppError::storage)?;
        }
        self.clients.lock().expect("not poisoned").clear();
        self.rooms.reset();
        Ok(RestoredView {
            name: contents.name.clone(),
            spaces: self.spaces()?,
        })
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
        self.seal_recovery_again();
        Ok(space_view(client.instance(), &view))
    }

    /// Every space this device knows, with its rooms and who sits in
    /// them. A server that cannot be reached does not hide the others: its
    /// spaces carry the error. Each space listed is then followed, so its
    /// occupants stay current ([`Desktop::spaces_now`]).
    pub fn spaces(&self) -> Result<Vec<SpaceView>, AppError> {
        let spaces = self.store.spaces().map_err(AppError::storage)?;
        let mut reached = Vec::new();
        let listing: Vec<SpaceView> = spaces
            .iter()
            .map(|known| {
                let client = self.client(&known.instance);
                let listed = client
                    .clone()
                    .and_then(|client| Ok(client.space(&known.space_id)?));
                match listed {
                    Ok(view) => {
                        // Keep the name the server knows now.
                        if view.space.name != known.name {
                            let mut renamed = known.clone();
                            renamed.name = view.space.name.clone();
                            let _ = self.store.remember(renamed);
                        }
                        if let Ok(client) = client {
                            reached.push((client, known.space_id));
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
            .collect();
        // The listing first: a change of occupants is shown with it.
        *self.listed.lock().expect("not poisoned") = listing;
        for (client, space) in &reached {
            self.rooms.follow(client, *space);
        }
        Ok(self.spaces_now())
    }

    /// The spaces as last listed, with who sits in their rooms now. Asks
    /// no server: what the window shows when someone comes or goes.
    pub fn spaces_now(&self) -> Vec<SpaceView> {
        let mut spaces = self.listed.lock().expect("not poisoned").clone();
        for space in &mut spaces {
            let Some(space_id) = Id::parse(&space.id) else {
                continue;
            };
            for room in &mut space.rooms {
                room.occupants = Id::parse(&room.id)
                    .and_then(|id| self.rooms.occupants(&space.instance, &space_id, &id));
            }
        }
        spaces
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
            settings: self.store.settings(),
            notices: Arc::clone(&self.notices),
        };
        // The call needs the sound card.
        self.close_meter();
        // The next join replaces this device's session on the server, so
        // there is no need to tell it we left.
        if let Some(previous) = self.take_session() {
            previous.end(false);
        }
        let session = Session::start(request, Arc::clone(&self.devices))?;
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

    /// Mutes or unmutes the microphone of the call, if one runs. Mute is
    /// not remembered: every call starts unmuted.
    pub fn set_muted(&self, muted: bool) {
        if let Some(session) = self.session.lock().expect("not poisoned").as_ref() {
            session.set_muted(muted);
        }
    }

    /// While the call is reconnecting, starts the waiting attempt now
    /// (ADR 0050).
    pub fn retry_now(&self) {
        if let Some(session) = self.session.lock().expect("not poisoned").as_ref() {
            session.retry_now();
        }
    }

    /// Mutes if unmuted, and the other way round; returns whether the
    /// microphone is now muted, or `None` out of a call.
    pub fn toggle_muted(&self) -> Option<bool> {
        let session = self.session.lock().expect("not poisoned");
        let session = session.as_ref()?;
        let muted = !session.view().muted;
        session.set_muted(muted);
        Some(muted)
    }

    /// The person's preferences.
    pub fn settings(&self) -> Settings {
        self.store.settings()
    }

    /// Saves the preferences the settings panel edits: the mute shortcut,
    /// the presence sounds' volume and the overlay. The gate and the
    /// volumes have their own methods, used during a call.
    pub fn set_preferences(&self, preferences: Preferences) -> Result<Settings, AppError> {
        let mut settings = self.store.settings();
        settings.mute_shortcut = preferences.mute_shortcut.trim().to_string();
        settings.sound_volume = if preferences.sound_volume.is_nan() {
            0.0
        } else {
            preferences.sound_volume.clamp(0.0, 1.0)
        };
        settings.overlay = preferences.overlay;
        if crate::store::OVERLAY_CORNERS.contains(&preferences.overlay_corner.as_str()) {
            settings.overlay_corner = preferences.overlay_corner;
        }
        self.store
            .save_settings(&settings)
            .map_err(AppError::storage)?;
        Ok(settings)
    }

    /// Moves the voice gate's threshold, now and for the next calls.
    pub fn set_gate(&self, open_dbfs: f32) -> Result<(), AppError> {
        let open_dbfs = clamp_gate(open_dbfs);
        let mut settings = self.store.settings();
        settings.gate_open_dbfs = Some(open_dbfs);
        self.store
            .save_settings(&settings)
            .map_err(AppError::storage)?;
        if let Some(session) = self.session.lock().expect("not poisoned").as_ref() {
            session.set_gate(open_dbfs);
        }
        Ok(())
    }

    /// Plays a person (by the hex of their public key) at `volume`, from 0
    /// to 2 (200 %), now and in every later call.
    pub fn set_volume(&self, public_key: &str, volume: f32) -> Result<(), AppError> {
        let volume = if volume.is_nan() {
            1.0
        } else {
            volume.clamp(0.0, drywire_core::mixer::MAX_VOLUME)
        };
        let mut settings = self.store.settings();
        if volume == 1.0 {
            settings.volumes.remove(public_key);
        } else {
            settings.volumes.insert(public_key.to_string(), volume);
        }
        self.store
            .save_settings(&settings)
            .map_err(AppError::storage)?;
        if let Some(session) = self.session.lock().expect("not poisoned").as_ref() {
            session.set_volume(public_key, volume);
        }
        Ok(())
    }

    /// The loopback test (spec section 11): clicks played on the default
    /// output and heard back by the default microphone, the earphone held
    /// against it, for at most `duration`. The median is kept for the
    /// latency estimates. Out of a call only: the call holds the device.
    pub fn loopback_test(&self, duration: Duration) -> Result<LoopbackView, AppError> {
        let _switching = self.switching.lock().expect("not poisoned");
        self.refuse_in_a_call()?;
        self.close_meter();
        let (test, input, output) = drywire_core::lab::Loopback::new();
        let mut device = (self.devices)()().map_err(|e| AppError::new("audio", e))?;
        let streams = device
            .start(input, output)
            .map_err(|e| AppError::new("audio", e.to_string()))?;
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline && test.delays_ms().len() < LOOPBACK_CLICKS {
            std::thread::sleep(Duration::from_millis(50));
        }
        device.stop();
        let median_ms = test.median_ms();
        if median_ms.is_some() {
            let mut settings = self.store.settings();
            settings.device_loopback_ms = median_ms;
            self.store
                .save_settings(&settings)
                .map_err(AppError::storage)?;
        }
        Ok(LoopbackView {
            median_ms,
            delays_ms: test.delays_ms(),
            clicks: test.clicks(),
            devices: streams.iter().map(DeviceView::new).collect(),
        })
    }

    /// Records the microphone for `duration`, the person silent but
    /// playing, and proposes a gate threshold just above that noise (spec
    /// section 10, "Gate calibration"). Out of a call only.
    pub fn calibrate(&self, duration: Duration) -> Result<CalibrationView, AppError> {
        let _switching = self.switching.lock().expect("not poisoned");
        self.refuse_in_a_call()?;
        self.close_meter();
        let frames = (duration.as_millis() / 5) as usize;
        let (recorder, input, output) = drywire_core::lab::LevelRecorder::new(frames);
        let mut device = (self.devices)()().map_err(|e| AppError::new("audio", e))?;
        device
            .start(input, output)
            .map_err(|e| AppError::new("audio", e.to_string()))?;
        // A little longer than asked: the device starts with a delay.
        let deadline = Instant::now() + duration + Duration::from_secs(2);
        while !recorder.is_full() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        device.stop();
        let found = drywire_core::gate::calibrate(&recorder.levels_dbfs())
            .ok_or_else(|| AppError::new("audio", "the microphone sent nothing"))?;
        Ok(CalibrationView {
            noise_dbfs: found.noise_dbfs,
            peak_dbfs: found.peak_dbfs,
            gate_dbfs: clamp_gate(found.gate.open_dbfs),
            too_loud: found.too_loud,
        })
    }

    /// The microphone's level for the settings' meter, in dBFS from
    /// [`METER_FLOOR_DBFS`] up. In a call, the call's. Out of one, the
    /// first request opens the sound card, and it closes by itself a
    /// second after the last ([`crate::meter`]). While a join or the lab
    /// holds the sound card, the floor.
    pub fn input_level(&self) -> Result<f32, AppError> {
        if let Some(call) = self.call() {
            return Ok(call.input_dbfs);
        }
        // Never wait for a join, which can take seconds: the interface
        // asks again a moment later.
        let Ok(_switching) = self.switching.try_lock() else {
            return Ok(METER_FLOOR_DBFS);
        };
        if let Some(call) = self.call() {
            return Ok(call.input_dbfs);
        }
        let mut meter = self.meter.lock().expect("not poisoned");
        if let Some(running) = meter.as_ref() {
            match running.level() {
                Err(why) => {
                    *meter = None;
                    return Err(AppError::new("audio", why));
                }
                Ok(level) if !running.is_finished() => return Ok(level),
                // It closed after a pause in the requests: open it again.
                Ok(_) => {}
            }
        }
        let started = Meter::start((self.devices)()).map_err(|e| AppError::new("audio", e))?;
        let level = started.level().map_err(|e| AppError::new("audio", e))?;
        *meter = Some(started);
        Ok(level)
    }

    /// Closes the meter's sound card, if open, and waits until it is.
    pub fn close_meter(&self) {
        self.meter.lock().expect("not poisoned").take();
    }

    fn refuse_in_a_call(&self) -> Result<(), AppError> {
        if self.call().is_some() {
            return Err(AppError::new(
                "busy",
                "leave the room first: the call is using the sound card",
            ));
        }
        Ok(())
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
                occupants: None,
            })
            .collect(),
        error: None,
    }
}
