// The Rust side's commands, typed. Each type mirrors a Rust struct in
// src-tauri/src (serde turns its fields into camelCase).
import { invoke } from "@tauri-apps/api/core";

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
  muted: boolean;
  /** The microphone's level for the meter, from -90 dBFS up. */
  inputDbfs: number;
  gateDbfs: number;
  uplinkLossPct: number;
  participants: Participant[];
  devices: DeviceView[];
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
};
