import { expect, test } from "bun:test";
import type { CallView, SpaceView } from "./api";
import { formatDb } from "./format";
import { findRoom, firstRoom, gateStatus, lockState, meFirst, othersFirst, signalOf, trustNames, trustOf, whereLabel } from "./model";
import { simulated } from "./simulated";

async function aCall(change: Partial<CallView> = {}): Promise<CallView> {
  const call = await simulated.joinRoom("voice.example.org", "team", "general");
  await simulated.leaveRoom();
  return { ...call, bound: true, rttMs: 14, uplinkLossPct: 0, ...change };
}

test("a room is named after its space when the space has only one", () => {
  expect(whereLabel("Team", "general", true)).toBe("Team · # general");
  expect(whereLabel("Friday night", "general", false)).toBe("Friday night");
});

test("the window starts on the first room of a space that has one", () => {
  const spaces: SpaceView[] = [
    { instance: "a", id: "down", name: "Down", role: "member", rooms: [], error: { code: "network", message: "" }, group: false },
    { instance: "b", id: "team", name: "Team", role: "member", rooms: [{ id: "general", name: "general" }], error: null, group: false },
  ];
  const first = firstRoom(spaces);
  expect(first).toEqual({ instance: "b", spaceId: "team", roomId: "general" });
  expect(findRoom(spaces, first)?.room.name).toBe("general");
  expect(findRoom(spaces, { instance: "b", spaceId: "team", roomId: "gone" })).toBeNull();
  expect(firstRoom([])).toBeNull();
});

test("the signal chip lights three bars for a good link and one for a bad one", async () => {
  expect(signalOf(await aCall())).toMatchObject({ bars: 3, poor: false, rttMs: 14 });
  expect(signalOf(await aCall({ rttMs: 182 }))).toMatchObject({ bars: 1, poor: true });
  expect(signalOf(await aCall({ uplinkLossPct: 3 }))).toMatchObject({ bars: 2, poor: false });
  // No round trip to show while connecting or reconnecting.
  expect(signalOf(await aCall({ bound: false }))).toMatchObject({ bars: 0, rttMs: null, connecting: true });
  expect(signalOf(await aCall({ reconnecting: true }))).toMatchObject({ bars: 0, connecting: false });
});

test("the gate sends above its threshold, never while muted", async () => {
  expect(gateStatus(await aCall({ inputDbfs: -40, gateDbfs: -58 }))).toBe("sending");
  expect(gateStatus(await aCall({ inputDbfs: -70, gateDbfs: -58 }))).toBe("below");
  expect(gateStatus(await aCall({ inputDbfs: -40, gateDbfs: -58, muted: true }))).toBe("muted");
});

test("the side menu lists you first, the call view last", () => {
  const people = [{ me: false, n: "sam" }, { me: true, n: "alex" }, { me: false, n: "kim" }];
  expect(meFirst(people).map((p) => p.n)).toEqual(["alex", "sam", "kim"]);
  expect(othersFirst(people).map((p) => p.n)).toEqual(["sam", "kim", "alex"]);
});

test("decibels are written with a true minus sign", () => {
  expect(formatDb(-58)).toBe("−58 dB");
  expect(formatDb(-57.6)).toBe("−58 dB");
  expect(formatDb(0)).toBe("0 dB");
});

test("the lock closes only when everyone heard is verified", async () => {
  const people = [{ publicKey: "a", verified: true }];
  expect(lockState(await aCall({ encryption: { state: "verified", people } }))).toBe("verified");
  expect(lockState(await aCall({ encryption: undefined }))).toBeNull();
  expect(lockState(await aCall({ encryption: { state: "pending", people } }))).toBe("pending");
  // The link down opens it, whatever the last state said.
  expect(lockState(await aCall({ reconnecting: true }))).toBe("pending");
  expect(lockState(await aCall({ bound: false }))).toBe("pending");
  expect(lockState(await aCall({ encryption: { state: "failed", people } }))).toBe("failed");
});

test("each person is verified, pending or rejected; this device is trusted", async () => {
  const call = await aCall();
  const [me, sam, kim] = call.participants;
  const withTrust = await aCall({
    encryption: {
      state: "failed",
      people: [
        { publicKey: sam.publicKey, verified: false, rejected: true },
        { publicKey: kim.publicKey, verified: false },
      ],
    },
  });
  expect(trustOf(withTrust, me)).toBe("ok");
  expect(trustOf(withTrust, sam)).toBe("failed");
  expect(trustOf(withTrust, kim)).toBe("pending");
  // lee is not in the list at all: unknown, so neither.
  expect(trustNames(withTrust)).toEqual({ failed: ["sam"], pending: ["kim"] });
  expect(trustOf(await aCall({ encryption: undefined }), me)).toBeNull();
});
