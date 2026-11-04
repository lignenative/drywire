import { expect, test } from "bun:test";
import { JitterBuffer, softClip } from "./jitter";

const N = 240; // 5 ms frames
const QUANTUM = 128;

/** A frame whose samples all hold its index, to see what plays. */
const frame = (i: number, end = false) => ({ ts: 10_000 + i * N, pcm: new Float32Array(N).fill(i + 1), end });

/** Plays `samples` from `now`, a quantum at a time; the values heard. */
function play(buffer: JitterBuffer, now: number, samples: number): Float32Array {
  const out = new Float32Array(samples);
  for (let at = 0; at < samples; at += QUANTUM) buffer.pull(out, at, Math.min(QUANTUM, samples - at), now + at, 1);
  return out;
}

test("a steady spurt plays every frame in order, one frame late", () => {
  const b = new JitterBuffer();
  const heard: number[] = [];
  // Frames arrive exactly on time; the clocks differ by a constant.
  for (let i = 0; i < 20; i++) {
    b.push(frame(i, i === 19), 50_000 + i * N);
    const out = play(b, 50_000 + i * N, N);
    heard.push(...out.filter((x) => x !== 0));
  }
  heard.push(...play(b, 50_000 + 20 * N, 4 * N).filter((x) => x !== 0));
  const order = heard.filter((x, i) => i === 0 || heard[i - 1] !== x);
  expect(order).toEqual(Array.from({ length: 20 }, (_, i) => i + 1));
  expect(b.stats).toMatchObject({ received: 20, played: 20, concealed: 0, late: 0, dropped: 0 });
  // No jitter: the target is its floor, one frame.
  expect(b.stats.targetSamples).toBe(N);
});

test("a lost frame is concealed and skipped, the timing kept", () => {
  const b = new JitterBuffer();
  for (let i = 0; i < 10; i++) {
    if (i !== 4) b.push(frame(i, i === 9), 50_000 + i * N);
    play(b, 50_000 + i * N, N);
  }
  play(b, 50_000 + 10 * N, 4 * N);
  expect(b.stats).toMatchObject({ received: 9, played: 9, concealed: 1, late: 0 });
});

test("a frame that comes after its turn plays late; one older than that is dropped", () => {
  const b = new JitterBuffer();
  b.push(frame(0), 50_000);
  // One frame of waiting, frame 0, then frame 1 is missing at its turn.
  play(b, 50_000, 3 * N);
  b.push(frame(1), 50_000 + 3 * N);
  play(b, 50_000 + 3 * N, N);
  expect(b.stats.played).toBe(2);
  expect(b.stats.concealed).toBeGreaterThan(0);
  b.push(frame(0), 50_000 + 4 * N); // a duplicate long after
  expect(b.stats.late).toBe(1);
});

test("a buffer too deep drops its oldest frame", () => {
  const b = new JitterBuffer();
  // Six frames at once: more than two frames above a one-frame target.
  for (let i = 0; i < 6; i++) b.push(frame(i), 50_000);
  expect(b.stats.dropped).toBeGreaterThan(0);
});

test("the target follows the jitter, at the start of the next spurt", () => {
  const b = new JitterBuffer();
  let now = 50_000;
  for (let i = 0; i < 40; i++) {
    // Every other frame is 3 ms late.
    b.push(frame(i, i === 39), 50_000 + i * N + (i % 2) * 144);
    play(b, now, N);
    now += N;
  }
  play(b, now, 10 * N);
  const next = { ts: 10_000 + 100 * N, pcm: new Float32Array(N), end: true };
  b.push(next, 50_000 + 100 * N);
  expect(b.stats.targetSamples).toBeGreaterThanOrEqual(144);
});

test("the soft clip leaves quiet sums alone and bounds loud ones", () => {
  for (const x of [-0.5, -0.1, 0, 0.25, 0.5]) expect(softClip(x)).toBe(x);
  expect(softClip(10)).toBeLessThanOrEqual(1);
  expect(softClip(-3)).toBe(-softClip(3));
});
