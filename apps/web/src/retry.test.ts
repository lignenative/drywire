import { expect, test } from "bun:test";
import { Retries, RETRY_S } from "./retry";

test("the pauses follow ADR 0035", () => {
  expect(RETRY_S).toEqual([1, 2, 4, 8, 15]);
});

// Pauses of 10, 20, 40 ms stand for 1, 2, 4 s.
const quick = () => new Retries([10, 20, 40]);

test("each wait is the next pause, the last one repeating", async () => {
  const retries = quick();
  const took: number[] = [];
  for (let i = 0; i < 4; i++) {
    const start = performance.now();
    expect(await retries.wait()).toBe(false);
    took.push(performance.now() - start);
  }
  expect(took[0]).toBeGreaterThanOrEqual(9);
  expect(took[1]).toBeGreaterThanOrEqual(19);
  expect(took[2]).toBeGreaterThanOrEqual(39);
  expect(took[3]).toBeGreaterThanOrEqual(39);
});

test("the countdown shows while waiting, not during an attempt", async () => {
  const retries = new Retries([1500]);
  expect(retries.secondsLeft()).toBeNull();
  const waiting = retries.wait();
  expect(retries.secondsLeft()).toBe(2);
  retries.now();
  await waiting;
  expect(retries.secondsLeft()).toBeNull();
});

test("trying now ends the wait at once", async () => {
  const retries = new Retries([60_000]);
  const waiting = retries.wait();
  expect(retries.now()).toBe(true);
  expect(await waiting).toBe(true);
});

test("a click during an attempt does nothing", async () => {
  const retries = quick();
  await retries.wait();
  // The attempt runs: nothing waits.
  expect(retries.now()).toBe(false);
  const start = performance.now();
  expect(await retries.wait()).toBe(false);
  expect(performance.now() - start).toBeGreaterThanOrEqual(19);
});

test("an attempt the person starts restarts the sequence", async () => {
  const retries = quick();
  await retries.wait();
  await retries.wait();
  const third = retries.wait();
  retries.now();
  expect(await third).toBe(true);
  // That attempt failed: the next pause is the first one again.
  const start = performance.now();
  expect(await retries.wait()).toBe(false);
  const took = performance.now() - start;
  expect(took).toBeGreaterThanOrEqual(9);
  expect(took).toBeLessThan(35);
});
