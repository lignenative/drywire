// What the page and its two worklets say to each other (engine.ts,
// capture.worklet.ts, playback.worklet.ts). Samples cross the thread
// boundary as transferred buffers, never copied twice.

/** To the capture worklet. */
export type ToCapture =
  | { type: "gate"; openDbfs: number }
  | { type: "muted"; muted: boolean }
  /** Records frame levels for the calibration, then sends them back. */
  | { type: "calibrate"; seconds: number };

/** From the capture worklet. */
export type FromCapture =
  /** A frame the gate lets through, to encode and send. */
  | { type: "frame"; pcm: Float32Array; ts: number; start: boolean; end: boolean }
  /** The loudest frame since the last report, for the meter, and whether
      the gate is open. */
  | { type: "level"; dbfs: number; open: boolean }
  | { type: "levels"; levels: number[] };

/** To the playback worklet. */
export type ToPlayback =
  | { type: "frame"; session: number; ts: number; end: boolean; pcm: Float32Array }
  | { type: "volume"; session: number; gain: number }
  | { type: "forget"; session: number };

export interface SenderStats {
  session: number;
  received: number;
  played: number;
  concealed: number;
  late: number;
  dropped: number;
  depth: number;
  targetMs: number;
  /** A talk spurt is playing or about to. */
  active: boolean;
}

/** From the playback worklet, ten times a second. */
export type FromPlayback = { type: "stats"; senders: SenderStats[] };
