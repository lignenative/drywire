// The Rust side's commands, typed. Each type mirrors a Rust struct in
// src-tauri/src (serde turns its fields into camelCase).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { simulated } from "./simulated";

/** Why a command failed: a code to translate, and the detail in English. */
export interface AppError {
  code: string;
  message: string;
}

export interface RoomView {
  id: string;
  name: string;
  /** Who sits in the room, sorted by name, from the space's event stream
      (the Rust side's `rooms.rs`). Absent when unknown: the stream is not
      open yet, or the instance has too many spaces to follow them all;
      the interface then shows the room without names. For the call's own
      room the interface shows the call's participants instead. */
  occupants?: string[];
}

export interface SpaceView {
  instance: string;
  id: string;
  name: string;
  role: string;
  rooms: RoomView[];
  error: AppError | null;
  /** Its keys come from an MLS group (ADRs 0052 and 0055): it invites by
      user name, and its links hold no key. */
  group: boolean;
}

/** What signing in by e-mail gave (ADR 0055). */
export interface SignedInView {
  /** The address had no account: it chooses its user name next. */
  newAccount: boolean;
  username: string | null;
}

/** The account as its settings page shows it (ADR 0055). */
export interface AccountInfoView {
  username: string;
  email: string;
  name: string;
}

export interface DeviceInfoView {
  publicKey: string;
  thisDevice: boolean;
  /** RFC 3339. */
  lastSeenAt: string;
}

/** An invitation by user name, waiting. */
export interface PendingInvitationView {
  instance: string;
  spaceId: string;
  spaceName: string;
  /** A user name; empty if that account is gone. */
  invitedBy: string;
}

export interface Participant {
  session: number;
  name: string;
  publicKey: string;
  me: boolean;
  speaking: boolean;
  /** 1 is 100 %, 2 the most. */
  volume: number;
  bufferMs: number | null;
  lossPct: number | null;
  /** Mouth to ear from them to this device: an estimate, always shown as one. */
  estimateMs: number | null;
  counters: Counters;
}

export interface Counters {
  received: number;
  played: number;
  concealed: number;
  late: number;
  dropped: number;
  depth: number;
}

export interface LoopbackView {
  medianMs: number | null;
  delaysMs: number[];
  clicks: number;
  devices: DeviceView[];
}

export interface CalibrationView {
  noiseDbfs: number;
  peakDbfs: number;
  gateDbfs: number;
  tooLoud: boolean;
}

export interface DeviceView {
  direction: "input" | "output";
  name: string;
  mode: string;
  bufferMs: number;
  latencyMs: number;
  bluetooth: boolean;
}

/** Whether everyone heard in the call is verified: their signed
    announcement matches the key the server gave, and their frames
    authenticate (ADR 0041 on the branch that adds it). */
export interface EncryptionView {
  /** "verified" once everyone heard is verified (silent people do not
      hold it back); "pending" while someone is not yet, alone, or
      reconnecting; "failed" for a few seconds after a forged announcement
      or a tampered frame, whose audio is dropped. */
  state: "pending" | "verified" | "failed";
  /** The others, in the participants' order. */
  people: EncryptionPerson[];
}

export interface EncryptionPerson {
  publicKey: string;
  verified: boolean;
  /** Whose voice was rejected: a packet in their name failed its signature
      or authentication lately, and was dropped. The Rust side always says;
      a client that cannot leaves it out, and the interface then words the
      failure without a name. */
  rejected?: boolean;
}

export interface CallView {
  instance: string;
  spaceId: string;
  spaceName: string;
  roomId: string;
  roomName: string;
  ended: string | null;
  /** The server forgot this session: the app is joining again on its own. */
  reconnecting: boolean;
  /** Seconds until the next attempt while reconnecting, rounded up
      (ADR 0035); null during an attempt and out of a reconnection. A
      client that cannot say leaves it out, and the interface then leaves
      the countdown out. */
  retryInS?: number | null;
  bound: boolean;
  rttMs: number | null;
  minRttMs: number | null;
  deviceMs: number;
  deviceMeasured: boolean;
  muted: boolean;
  /** The microphone's level for the meter, from -90 dBFS up. */
  inputDbfs: number;
  gateDbfs: number;
  /** Whether the person chose the threshold, or it is still the default. */
  gateChosen: boolean;
  uplinkLossPct: number;
  participants: Participant[];
  devices: DeviceView[];
  /** The encryption lock's state, from the core (ADR 0041). */
  encryption: EncryptionView;
}

/** The person's preferences (settings.json). */
export interface Settings {
  /** Null until the person calibrates or moves the slider. */
  gateOpenDbfs: number | null;
  volumes: Record<string, number>;
  muteShortcut: string;
  /** 0 (off) to 1. */
  soundVolume: number;
  overlay: boolean;
  overlayCorner: OverlayCorner;
}

/** Where the overlay sits on the main screen. */
export type OverlayCorner = "top-left" | "top-right" | "bottom-left" | "bottom-right";

/** What the settings panel edits. */
export interface Preferences {
  muteShortcut: string;
  soundVolume: number;
  overlay: boolean;
  overlayCorner: OverlayCorner;
}

/** Someone came into the room, or went. */
export interface Notice {
  kind: "join" | "leave" | "disconnect";
  name: string;
}

/** The account as the settings show it (ADR 0051). */
export interface AccountView {
  /** Whether this device knows the account's current recovery code. */
  recoveryCode: boolean;
  /** How many spaces it has: none, and no instance keeps a vault yet. */
  spaces: number;
  /** Whether its key can go in a vault: not an old browser key. */
  portable: boolean;
}

/** What opening a code brought: the account's name and its spaces. */
export interface RestoredView {
  name: string;
  spaces: SpaceView[];
}

/** An invitation link made for a space: 10 uses, 7 days. */
export interface InvitationView {
  /** The link, which holds the space key: shown, never logged. */
  link: string;
  uses: number;
  expiresAt: string;
}

/** A device code, waiting on an instance for ten minutes. */
export interface DeviceCodeView {
  code: string;
  instance: string;
  expiresInS: number;
}

/** Whether a value thrown by a command is an AppError. */
export function isAppError(value: unknown): value is AppError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as AppError).code === "string" &&
    typeof (value as AppError).message === "string"
  );
}

/** The Rust side, inside the Tauri window. */
const tauri = {
  publicKey: () => invoke<string>("public_key"),
  account: () => invoke<AccountView>("account"),
  /** A new recovery code, which replaces the old one: show it once. */
  newRecoveryCode: () => invoke<string>("new_recovery_code"),
  newDeviceCode: () => invoke<DeviceCodeView>("new_device_code"),
  /** Becomes the account a code opens on `instance`. Refused with
      `account_exists` when this device already uses another account in
      some space, unless `replace`. Returns its name and its spaces, as
      listSpaces gives them. */
  restoreAccount: (instance: string, code: string, replace: boolean) =>
    invoke<RestoredView>("restore_account", { instance, code, replace }),
  /** Creates a space, with a creation code from the instance's operator
      when this account is not one it lets create spaces (ADR 0054). */
  createSpace: (instance: string, creatorCode: string | null, name: string, displayName: string) =>
    invoke<SpaceView>("create_space", { instance, creatorCode, name, displayName }),
  /** An invitation link to a space, for owners and admins. */
  createInvitation: (instance: string, spaceId: string) =>
    invoke<InvitationView>("create_invitation", { instance, spaceId }),
  acceptInvitation: (link: string, displayName: string) =>
    invoke<SpaceView>("accept_invitation", { link, displayName }),
  listSpaces: () => invoke<SpaceView[]>("list_spaces"),
  joinRoom: (instance: string, spaceId: string, roomId: string) =>
    invoke<CallView>("join_room", { instance, spaceId, roomId }),
  leaveRoom: () => invoke<void>("leave_room"),
  callStatus: () => invoke<CallView | null>("call_status"),
  setMuted: (muted: boolean) => invoke<void>("set_muted", { muted }),
  /** While reconnecting, starts the waiting attempt now instead of after
      the countdown (ADR 0050). Does nothing during an attempt. */
  retryNow: () => invoke<void>("retry_now"),
  setGate: (openDbfs: number) => invoke<void>("set_gate", { openDbfs }),
  setVolume: (publicKey: string, volume: number) =>
    invoke<void>("set_volume", { publicKey, volume }),
  getSettings: () => invoke<Settings>("get_settings"),
  setPreferences: (preferences: Preferences) =>
    invoke<Settings>("set_preferences", { preferences }),
  setLanguage: (language: string) => invoke<void>("set_language", { language }),
  loopbackTest: () => invoke<LoopbackView>("loopback_test"),
  calibrateGate: () => invoke<CalibrationView>("calibrate_gate"),
  /** The microphone's level, in dBFS from -90 up: the call's in a call.
      Out of one, the first request opens the microphone, and the Rust side
      closes it a second after the last: poll it only while a meter shows. */
  inputLevel: () => invoke<number>("input_level"),
  launchAtLogin: () => invoke<boolean>("launch_at_login"),
  setLaunchAtLogin: (enabled: boolean) => invoke<boolean>("set_launch_at_login", { enabled }),
  /** The instance this build signs in to without asking; empty if none
      (ADR 0055). */
  defaultInstance: () => invoke<string>("default_instance"),
  /** Mails a sign-in code to the address. */
  emailCode: (instance: string, email: string, lang: string) =>
    invoke<void>("email_code", { instance, email, lang }),
  /** Signs this device in with the mailed code: `wrong_code` with the
      tries left as the message when it does not match. */
  emailSignIn: (instance: string, email: string, code: string) =>
    invoke<SignedInView>("email_sign_in", { instance, email, code }),
  /** Chooses the user name: `username_taken` with free ones,
      comma-separated, as the message. */
  setUsername: (instance: string, username: string, displayName: string) =>
    invoke<string>("set_username", { instance, username, displayName }),
  pendingInvitations: (instance: string) =>
    invoke<PendingInvitationView[]>("pending_invitations", { instance }),
  answerInvitation: (instance: string, spaceId: string, accept: boolean) =>
    invoke<SpaceView | null>("answer_invitation", { instance, spaceId, accept }),
  /** Invites someone by user name; their display name. */
  inviteUser: (instance: string, spaceId: string, username: string) =>
    invoke<string>("invite_user", { instance, spaceId, username }),
  accountInfo: (instance: string) => invoke<AccountInfoView>("account_info", { instance }),
  accountDevices: (instance: string) => invoke<DeviceInfoView[]>("account_devices", { instance }),
  removeDevice: (instance: string, publicKey: string) => invoke<void>("remove_device", { instance, publicKey }),
  /** Signs this device out: the account forgets it, and it forgets the
      account's spaces on `instance`. */
  signOut: (instance: string) => invoke<void>("sign_out", { instance }),
  /** Calls `f` with the spaces whenever someone sits down in or leaves a
      room of any of them: what `listSpaces` would answer, without asking
      the servers again. */
  onSpaces: (f: (spaces: SpaceView[]) => void): Promise<UnlistenFn> =>
    listen<SpaceView[]>("spaces", (event) => f(event.payload)),
  /** Calls `f` for every arrival and departure in the room. */
  onPresence: (f: (notice: Notice) => void): Promise<UnlistenFn> =>
    listen<Notice>("presence", (event) => f(event.payload)),
};

/** Every command the interface sends. */
export type Api = typeof tauri;

/** Whether the page runs in the Tauri window, not in a plain browser. */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Where the interface runs. "app" is the desktop app, or its simulated
    call in a browser (`make ui`); "web" is the web client (apps/web),
    which has no overlay, global shortcut, launch at login or loopback
    test, so the settings leave them out. */
export type Platform = "app" | "web";

/** The Rust side in the app; a simulated call in a browser (`make ui`).
    A binding, not a constant: the web client hands in its own. */
export let api: Api = inTauri ? tauri : simulated;
export let platform: Platform = "app";

/** Gives the interface another implementation of its commands, before
    its first render: the web client's, built on WebTransport, WebCodecs
    and the protocol in WebAssembly (ADR 0047). */
export function provideApi(next: Api, where: Platform): void {
  api = next;
  platform = where;
}
