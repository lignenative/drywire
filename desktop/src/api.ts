// The Rust side's commands, typed. Each type mirrors a Rust struct in
// src-tauri/src (serde turns its fields into camelCase).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Why a command failed: a code to translate, and the detail in English. */
export interface AppError {
  code: string;
  message: string;
}

export interface RoomView {
  id: string;
  name: string;
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

export interface CallView {
  instance: string;
  spaceId: string;
  spaceName: string;
  roomId: string;
  roomName: string;
  ended: string | null;
  bound: boolean;
  rttMs: number | null;
  minRttMs: number | null;
  deviceMs: number;
  deviceMeasured: boolean;
  muted: boolean;
  /** The microphone's level for the meter, from -90 dBFS up. */
  inputDbfs: number;
  gateDbfs: number;
  uplinkLossPct: number;
  participants: Participant[];
  devices: DeviceView[];
}

/** The person's preferences (settings.json). */
export interface Settings {
  gateOpenDbfs: number;
  volumes: Record<string, number>;
  muteShortcut: string;
  /** 0 (off) to 1. */
  soundVolume: number;
  overlay: boolean;
}

/** What the settings panel edits. */
export interface Preferences {
  muteShortcut: string;
  soundVolume: number;
  overlay: boolean;
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

export const api = {
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
  /** Calls `f` for every arrival and departure in the room. */
  onPresence: (f: (notice: Notice) => void): Promise<UnlistenFn> =>
    listen<Notice>("presence", (event) => f(event.payload)),
};
