import { expect, test } from "bun:test";
import { simulated } from "./simulated";

test("the simulated call answers like the Rust side", async () => {
  expect(await simulated.callStatus()).toBeNull();
  const spaces = await simulated.listSpaces();
  expect(spaces.map((s) => s.name)).toEqual(["Team", "Friday night", "Old crew"]);

  const call = await simulated.joinRoom("voice.example.org", "team", "general");
  expect(call.roomName).toBe("general");
  expect(call.participants.map((p) => p.name)).toEqual(["alex", "sam", "kim", "lee"]);
  expect(call.participants.filter((p) => p.me)).toHaveLength(1);
  // Others carry an estimate; this device never does.
  expect(call.participants.find((p) => p.me)?.estimateMs).toBeNull();

  await simulated.setMuted(true);
  const muted = await simulated.callStatus();
  expect(muted?.muted).toBe(true);
  expect(muted?.participants.find((p) => p.me)?.speaking).toBe(false);

  await simulated.leaveRoom();
  expect(await simulated.callStatus()).toBeNull();
});

test("an unknown space is refused with an app error", async () => {
  await expect(simulated.joinRoom("nowhere.example", "x", "y")).rejects.toMatchObject({ code: "not_found" });
});

test("the gate stays the default until chosen", async () => {
  expect((await simulated.getSettings()).gateOpenDbfs).toBeNull();
  await simulated.setGate(-50);
  expect((await simulated.getSettings()).gateOpenDbfs).toBe(-50);
});
