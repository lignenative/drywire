import { expect, test } from "bun:test";
import { calibrate, DEFAULT_GATE, levelDbfs, VoiceGate } from "./gate";

// The same cases as core/drywire-core/src/gate.rs, with 5 ms frames.
const at = (dbfs: number) => levelDbfs(new Float32Array(240).fill(10 ** (dbfs / 20)));

test("the level of known signals", () => {
  expect(at(-20)).toBeCloseTo(-20, 2);
  expect(levelDbfs(new Float32Array(4))).toBe(-Infinity);
});

test("opens on the first loud frame, holds, then ends the spurt", () => {
  const gate = new VoiceGate(DEFAULT_GATE, 5);
  expect(gate.process(at(-60))).toBe("silent");
  expect(gate.process(at(-30))).toBe("start");
  for (let i = 0; i < 29; i++) expect(gate.process(at(-70))).toBe("send");
  expect(gate.process(at(-70))).toBe("end");
  expect(gate.process(at(-70))).toBe("silent");
});

test("hysteresis keeps an open gate open between the thresholds", () => {
  const gate = new VoiceGate(DEFAULT_GATE, 5);
  const between = at((DEFAULT_GATE.openDbfs + DEFAULT_GATE.closeDbfs) / 2);
  expect(gate.process(between)).toBe("silent");
  gate.process(at(-30));
  for (let i = 0; i < 1000; i++) expect(gate.process(between)).toBe("send");
});

test("calibration proposes a threshold above the noise's peaks", () => {
  const levels = Array.from({ length: 100 }, (_, i) => (i >= 98 ? -55 : -67));
  expect(calibrate(levels)).toEqual({ noiseDbfs: -67, peakDbfs: -55, gateDbfs: -51, tooLoud: false });
  expect(calibrate(Array(10).fill(-10))?.tooLoud).toBe(true);
  expect(calibrate([])).toBeNull();
});
