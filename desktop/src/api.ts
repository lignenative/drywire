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
  /** Who sits in the room, for a room this device is not in. Absent when
      unknown: the Rust side does not report it yet (TODO: a command that
      reads the room's presence from the server), and the interface then
      shows the room without names. */
  occupants?: string[];
}

export interface SpaceView {
  instance: string;
  id: string;
  name: string;
  role: string;
  rooms: RoomView[];
  error: AppError | null;
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
  acceptInvitation: (link: string, displayName: string) =>
    invoke<SpaceView>("accept_invitation", { link, displayName }),
  listSpaces: () => invoke<SpaceView[]>("list_spaces"),
  joinRoom: (instance: string, spaceId: string, roomId: string) =>
    invoke<CallView>("join_room", { instance, spaceId, roomId }),
  leaveRoom: () => invoke<void>("leave_room"),
  callStatus: () => invoke<CallView | null>("call_status"),
  setMuted: (muted: boolean) => invoke<void>("set_muted", { muted }),
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
  /** Calls `f` for every arrival and departure in the room. */
  onPresence: (f: (notice: Notice) => void): Promise<UnlistenFn> =>
    listen<Notice>("presence", (event) => f(event.payload)),
};

/** Every command the interface sends. */
export type Api = typeof tauri;

/** Whether the page runs in the Tauri window, not in a plain browser. */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** The Rust side in the app; a simulated call in a browser (`make ui`). */
export const api: Api = inTauri ? tauri : simulated;
