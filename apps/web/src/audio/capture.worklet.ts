// The microphone's side of the call, on the audio thread: it cuts the
// input into Opus-sized frames, measures each one, and lets the voice gate
// decide (gate.ts). Frames the gate lets through go to the page, which
// encodes them with WebCodecs, absent from worklets (ADR 0040). Nothing
// here changes a sample.
import { DEFAULT_GATE, gateAt, gateFrame, VoiceGate } from "./gate";
import type { FromCapture, ToCapture } from "./messages";

/** Frames between two level reports: 50 ms of 5 ms frames. */
const LEVEL_EVERY = 10;

class Capture extends AudioWorkletProcessor {
  private readonly frame: Float32Array;
  private filled = 0;
  /** The timestamp of the frame being filled, in samples. It keeps
      advancing while the gate is closed (protocol section 6.6). */
  private ts = Math.floor(Math.random() * 2 ** 31);
  private readonly gate: VoiceGate;
  private muted = false;
  private loudest = -Infinity;
  private frames = 0;
  private calibration: number[] | null = null;
  private calibrationLeft = 0;

  constructor(options: AudioWorkletNodeOptions) {
    super();
    const samples = (options.processorOptions as { frameSamples: number }).frameSamples;
    this.frame = new Float32Array(samples);
    this.gate = new VoiceGate(DEFAULT_GATE, (samples * 1000) / sampleRate);
    this.port.onmessage = (e: MessageEvent<ToCapture>) => {
      const m = e.data;
      if (m.type === "gate") this.gate.setConfig(gateAt(m.openDbfs));
      else if (m.type === "muted") this.muted = m.muted;
      else if (m.type === "calibrate") {
        this.calibration = [];
        this.calibrationLeft = Math.round((m.seconds * sampleRate) / this.frame.length);
      }
    };
  }

  process(inputs: Float32Array[][]): boolean {
    const input = inputs[0]?.[0];
    if (!input) return true;
    let at = 0;
    while (at < input.length) {
      const take = Math.min(input.length - at, this.frame.length - this.filled);
      this.frame.set(input.subarray(at, at + take), this.filled);
      this.filled += take;
      at += take;
      if (this.filled === this.frame.length) {
        this.filled = 0;
        this.onFrame();
      }
    }
    // The start of the next frame, captured while muted: silenced now, or
    // an unmute before the frame completes would send it (as call.rs).
    if (this.muted) this.frame.fill(0, 0, this.filled);
    return true;
  }

  private onFrame(): void {
    const { level, decision } = gateFrame(this.gate, this.frame, this.muted);
    if (decision !== "silent") {
      const pcm = this.frame.slice();
      this.post({ type: "frame", pcm, ts: this.ts, start: decision === "start", end: decision === "end" }, [pcm.buffer]);
    }
    this.ts += this.frame.length;

    this.loudest = Math.max(this.loudest, level);
    if (++this.frames % LEVEL_EVERY === 0) {
      this.post({ type: "level", dbfs: Math.max(this.loudest, -90), open: this.gate.isOpen() && !this.muted });
      this.loudest = -Infinity;
    }
    if (this.calibration) {
      this.calibration.push(level);
      if (--this.calibrationLeft <= 0) {
        this.post({ type: "levels", levels: this.calibration });
        this.calibration = null;
      }
    }
  }

  private post(message: FromCapture, transfer: Transferable[] = []): void {
    this.port.postMessage(message, transfer);
  }
}

registerProcessor("drywire-capture", Capture);
