// The voice gate and its calibration, as the native core has them
// (core/drywire-core/src/gate.rs): the same thresholds, hold and rules, so
// a person's gate behaves the same in a browser and in the app. It only
// decides whether a frame is sent; it never changes a sample.
//
// It runs in the capture worklet, on the audio thread: no allocation per
// frame.

/** Thresholds and hold of a gate, as GateConfig in the core. */
export interface GateConfig {
  /** Level, in dBFS, at or above which a closed gate opens. */
  openDbfs: number;
  /** Level at or above which an open gate restarts its hold. */
  closeDbfs: number;
  holdMs: number;
}

/** The core's defaults: lean towards sending until calibrated (ADR 0034). */
export const DEFAULT_GATE: GateConfig = { openDbfs: -50, closeDbfs: -56, holdMs: 150 };

/** How far below the open threshold the gate closes, as in the app. */
export const HYSTERESIS_DB = 6;

/** A gate that opens at `openDbfs`, with the app's hysteresis and hold. */
export function gateAt(openDbfs: number): GateConfig {
  return { openDbfs, closeDbfs: openDbfs - HYSTERESIS_DB, holdMs: DEFAULT_GATE.holdMs };
}

/** What to do with one frame. */
export type Decision = "silent" | "start" | "send" | "end";

export class VoiceGate {
  private holdFrames = 1;
  /** Frames left before closing; 0 means closed. */
  private remaining = 0;

  constructor(
    private config: GateConfig,
    private readonly frameMs: number,
  ) {
    this.setConfig(config);
  }

  /** New thresholds from the next frame on; an open gate stays open, with
      at most the new hold left. */
  setConfig(config: GateConfig): void {
    this.config = { ...config, closeDbfs: Math.min(config.closeDbfs, config.openDbfs) };
    this.holdFrames = Math.max(1, Math.ceil(config.holdMs / this.frameMs));
    this.remaining = Math.min(this.remaining, this.holdFrames);
  }

  isOpen(): boolean {
    return this.remaining > 0;
  }

  /** Decides for the next frame, from its level. "start" is the first
      frame of a talk spurt, "end" the last one, still sent. */
  process(levelDbfs: number): Decision {
    if (!this.isOpen()) {
      if (levelDbfs < this.config.openDbfs) return "silent";
      this.remaining = this.holdFrames;
      return "start";
    }
    if (levelDbfs >= this.config.closeDbfs) this.remaining = this.holdFrames;
    else this.remaining--;
    return this.remaining === 0 ? "end" : "send";
  }
}

/** One captured frame through the gate. A muted frame is heard as
    silence, as the core does it (call.rs): its samples become zeros, never
    sent as they were, and an open gate closes the spurt with its hold and
    its end mark, so the others stop at once instead of concealing a
    missing end for a fifth of a second. The level returned is the
    microphone's own, for the meter. */
export function gateFrame(gate: VoiceGate, frame: Float32Array, muted: boolean): { level: number; decision: Decision } {
  const level = levelDbfs(frame);
  if (muted) frame.fill(0);
  return { level, decision: gate.process(muted ? -Infinity : level) };
}

/** The RMS level of a frame in dBFS: −3 for a full-scale sine, −Infinity
    for digital silence. */
export function levelDbfs(frame: Float32Array): number {
  if (frame.length === 0) return -Infinity;
  let sum = 0;
  for (let i = 0; i < frame.length; i++) sum += frame[i] * frame[i];
  return 10 * Math.log10(sum / frame.length);
}

/** As in the core: the gate opens this far above the noise's peaks… */
export const CALIBRATION_MARGIN_DB = 4;
/** …and stays in this range. */
export const CALIBRATION_RANGE_DBFS: readonly [number, number] = [-70, -20];

export interface Calibration {
  noiseDbfs: number;
  peakDbfs: number;
  gateDbfs: number;
  tooLoud: boolean;
}

/** The threshold for a microphone whose background noise had these
    per-frame levels: the core's `calibrate`, step for step. */
export function calibrate(levels: number[]): Calibration | null {
  if (levels.length === 0) return null;
  const sorted = levels.map((l) => Math.max(l, -120)).sort((a, b) => a - b);
  const at = (q: number) => sorted[Math.round((sorted.length - 1) * q)];
  const noiseDbfs = at(0.5);
  const peakDbfs = at(0.99);
  const wanted = peakDbfs + CALIBRATION_MARGIN_DB;
  const [low, high] = CALIBRATION_RANGE_DBFS;
  return {
    noiseDbfs,
    peakDbfs,
    gateDbfs: Math.min(Math.max(wanted, low), high),
    tooLoud: wanted > high,
  };
}
