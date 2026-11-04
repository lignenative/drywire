// What the screens derive from the Rust side's views, kept apart from the
// components so it can be tested without a browser.
import type { CallView, Participant, RoomView, SpaceView } from "./api";
import { linkQuality } from "./format";

/** One room of one space, the way the screens point at it. */
export interface RoomRef {
  instance: string;
  spaceId: string;
  roomId: string;
}

export function sameRoom(a: RoomRef | null, b: RoomRef | null): boolean {
  return !!a && !!b && a.instance === b.instance && a.spaceId === b.spaceId && a.roomId === b.roomId;
}

export function refOf(space: SpaceView, room: RoomView): RoomRef {
  return { instance: space.instance, spaceId: space.id, roomId: room.id };
}

/** The room a call is in. */
export function callRoom(call: CallView): RoomRef {
  return { instance: call.instance, spaceId: call.spaceId, roomId: call.roomId };
}

/** A call the server ended still exists until it is closed, but this
    device is no longer in it. */
export function inCall(call: CallView | null): call is CallView {
  return !!call && !call.ended;
}

/** A space with one room is a single row named after the space; only a
    space with several rooms lists them under its name (the canvas's rule:
    the space is the main label). */
export function isMultiRoom(space: SpaceView): boolean {
  return space.rooms.length > 1;
}

/** Where a room is, in one line: "Team · # general", or "Friday night"
    for a space with a single room. */
export function whereLabel(spaceName: string, roomName: string, multi: boolean): string {
  return multi ? `${spaceName} · # ${roomName}` : spaceName;
}

export function findRoom(
  spaces: SpaceView[],
  ref: RoomRef | null,
): { space: SpaceView; room: RoomView } | null {
  if (!ref) return null;
  const space = spaces.find((s) => s.instance === ref.instance && s.id === ref.spaceId);
  const room = space?.rooms.find((r) => r.id === ref.roomId);
  return space && room ? { space, room } : null;
}

/** The first room of the first space that has one: what the window shows
    before the person picks anything. */
export function firstRoom(spaces: SpaceView[]): RoomRef | null {
  for (const space of spaces) {
    if (space.rooms.length > 0) return refOf(space, space.rooms[0]);
  }
  return null;
}

/** The connection chip's state: how many of its three bars are lit, and
    whether the link is bad enough to colour it. */
export interface Signal {
  bars: 0 | 1 | 2 | 3;
  poor: boolean;
  /** The round trip in milliseconds, or null while there is none. */
  rttMs: number | null;
  connecting: boolean;
}

export function signalOf(call: CallView): Signal {
  const up = call.bound && !call.reconnecting && !call.ended;
  if (!up) return { bars: 0, poor: false, rttMs: null, connecting: !call.reconnecting && !call.ended };
  const quality = linkQuality(call.rttMs, call.uplinkLossPct);
  return {
    bars: quality === "good" ? 3 : quality === "fair" ? 2 : 1,
    poor: quality === "poor",
    rttMs: call.rttMs,
    connecting: false,
  };
}

/** What the voice gate does right now. */
export type GateStatus = "muted" | "sending" | "below";

export function gateStatus(call: CallView, gateDbfs = call.gateDbfs): GateStatus {
  if (call.muted) return "muted";
  return call.bound && call.inputDbfs >= gateDbfs ? "sending" : "below";
}

/** The people of the room, this device first: the order the canvas lists
    them in the side menu, while the call view lists the others first. */
export function meFirst<T extends { me: boolean }>(people: T[]): T[] {
  return [...people.filter((p) => p.me), ...people.filter((p) => !p.me)];
}

export function othersFirst<T extends { me: boolean }>(people: T[]): T[] {
  return [...people.filter((p) => !p.me), ...people.filter((p) => p.me)];
}

/** What the encryption lock says, or null while the Rust side does not
    report it: then no lock is drawn rather than a wrong one. */
export type LockState = "verified" | "pending" | "failed";

export function lockState(call: CallView): LockState | null {
  if (!call.encryption) return null;
  if (call.encryption.state === "failed") return "failed";
  // Nothing is verified while the link is down: the canvas opens the lock.
  if (!call.bound || call.reconnecting) return "pending";
  return call.encryption.state;
}

/** One person's standing: verified, still pending, or rejected (their
    audio is dropped). This device is always trusted. Null when unknown. */
export type Trust = "ok" | "pending" | "failed";

export function trustOf(call: CallView, person: Participant): Trust | null {
  if (person.me) return call.encryption ? "ok" : null;
  const entry = call.encryption?.people.find((p) => p.publicKey === person.publicKey);
  if (!entry) return null;
  if (entry.rejected) return "failed";
  return entry.verified ? "ok" : "pending";
}

/** The names of the people whose voice was rejected, and of those still
    waiting, for the lock's sentence. */
export function trustNames(call: CallView): { failed: string[]; pending: string[] } {
  const failed: string[] = [];
  const pending: string[] = [];
  for (const p of call.participants) {
    const trust = trustOf(call, p);
    if (trust === "failed") failed.push(p.name);
    if (trust === "pending") pending.push(p.name);
  }
  return { failed, pending };
}
