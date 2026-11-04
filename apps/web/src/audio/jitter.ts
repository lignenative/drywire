// The jitter buffer of the web client: one per sender, in the playback
// worklet. It follows the native one (core/drywire-core/src/jitter.rs,
// spec section 7 "Reception") on decoded samples instead of Opus frames,
// because the browser decodes Opus on the main thread (WebCodecs is not
// available in a worklet, ADR 0040):
//
// - Frames are ordered by their timestamp, which keeps advancing through
//   silences, so a gap in timestamps is a gap in sound.
// - Every frame's transit is its arrival, on the audio clock, minus its
//   timestamp. The smallest recent transit is "on time"; how far a frame
//   lags behind it is its jitter.
// - The target delay is the 95th percentile of recent jitter plus a
//   margin, never less than one frame. It changes only when a talk spurt
//   starts: the spurt's first frame plays at its on-time arrival plus the
//   target, the next ones one after the other at the audio clock's pace.
// - Within a spurt, a frame missing when its turn comes is concealed with
//   silence (WebCodecs has no packet loss concealment) and waited for: if
//   it arrives, it plays late and the spurt stretches; if a later frame
//   arrives first, it was lost and is skipped. More than two frames above
//   the target and the oldest is dropped. Everything is counted.
//
// Time is in samples at 48 kHz, from the caller: the tests drive it with
// simulated time, the worklet with `currentFrame`.

/** One decoded frame from a sender. */
export interface Frame {
  /** The sender's timestamp of its first sample, unwrapped to a number. */
  ts: number;
  pcm: Float32Array;
  /** The last frame of a talk spurt (protocol section 5). */
  end: boolean;
}

export interface JitterStats {
  received: number;
  played: number;
  concealed: number;
  late: number;
  dropped: number;
  /** Frames waiting, now. */
  depth: number;
  /** The current target delay, in samples. */
  targetSamples: number;
  /** Whether a talk spurt is playing or about to. */
  active: boolean;
}

/** Recent transits kept for the target: about four seconds of 5 ms frames. */
const WINDOW = 800;
/** Added to the 95th percentile of jitter: half a millisecond. A frame
    arrives on a render quantum's boundary (128 samples), which the
    percentile already sees. */
const MARGIN = 24;
/** A spurt whose frames stop coming without an end mark is over after
    this many samples of concealment: 200 ms. */
const GIVE_UP = 9600;
/** A frame this far ahead of the one expected starts a new spurt: 300 ms. */
const RESYNC = 14400;

export class JitterBuffer {
  private queue: Frame[] = [];
  private transits = new Float64Array(WINDOW);
  private transitCount = 0;
  private transitAt = 0;
  private scratch = new Float64Array(WINDOW);

  private state: "idle" | "waiting" | "playing" = "idle";
  /** Local time at which the waiting spurt starts. */
  private playAt = 0;
  /** The sender timestamp due next. */
  private cursor = 0;
  /** The frame playing, and how much of it has been read. */
  private current: Float32Array | null = null;
  private read = 0;
  private currentEnds = false;
  /** The last frame length seen: what a concealed frame lasts. */
  private frameLength = 240;
  /** Concealed samples in a row. */
  private concealing = 0;
  private target = 240;

  readonly stats: JitterStats = {
    received: 0,
    played: 0,
    concealed: 0,
    late: 0,
    dropped: 0,
    depth: 0,
    targetSamples: 240,
    active: false,
  };

  /** A frame arrived at local time `now`. */
  push(frame: Frame, now: number): void {
    this.stats.received++;
    this.frameLength = frame.pcm.length;
    this.transits[this.transitAt] = now - frame.ts;
    this.transitAt = (this.transitAt + 1) % WINDOW;
    this.transitCount = Math.min(this.transitCount + 1, WINDOW);

    if (this.state !== "idle" && frame.ts < this.cursor) {
      this.stats.late++;
      return;
    }
    if (this.state === "playing" && frame.ts - this.cursor > RESYNC) {
      // The sender moved on without us (an end mark lost, a long stall):
      // what waits is stale.
      this.stats.dropped += this.queue.length;
      this.queue.length = 0;
      this.stop();
    }
    this.insert(frame);
    if (this.state === "idle") this.startSpurt();
    // Too deep: drop the oldest waiting frame, the spurt catches up.
    while (this.queued() > this.target + 2 * this.frameLength && this.queue.length > 1) {
      const old = this.queue.shift()!;
      this.stats.dropped++;
      if (old.ts <= this.cursor) this.cursor = old.ts + old.pcm.length;
    }
    this.stats.depth = this.queue.length;
  }

  /** Adds this sender's next `n` samples, times `gain`, into `out` from
      `offset`, for the samples that play from local time `now`. */
  pull(out: Float32Array, offset: number, n: number, now: number, gain: number): void {
    let i = 0;
    while (i < n) {
      if (this.state === "idle") break;
      if (this.state === "waiting") {
        const wait = this.playAt - (now + i);
        if (wait >= n - i) break;
        if (wait > 0) i += wait;
        this.state = "playing";
      }
      if (this.current === null || this.read >= this.current.length) {
        // Over: idle, or the next spurt already waiting.
        if (!this.next()) continue;
      }
      const frame = this.current!;
      const take = Math.min(n - i, frame.length - this.read);
      if (gain !== 0) {
        for (let k = 0; k < take; k++) out[offset + i + k] += frame[this.read + k] * gain;
      }
      this.read += take;
      i += take;
    }
    this.stats.depth = this.queue.length;
    this.stats.targetSamples = this.target;
    this.stats.active = this.state !== "idle";
  }

  /** Moves to the next frame; false when the spurt is over. */
  private next(): boolean {
    if (this.currentEnds) {
      this.stop();
      return false;
    }
    // Frames that came too late for their turn.
    while (this.queue.length > 0 && this.queue[0].ts < this.cursor) {
      this.queue.shift();
      this.stats.late++;
    }
    const head = this.queue[0];
    if (head && head.ts === this.cursor) {
      this.queue.shift();
      this.play(head);
      return true;
    }
    if (head && this.concealing > 0 && head.ts > this.cursor) {
      // A later frame came first: the one awaited was lost. Skip what the
      // concealment already covered, keeping the spurt's timing.
      const skipped = this.cursor + this.concealing;
      if (head.ts <= skipped) {
        this.cursor = head.ts;
        this.queue.shift();
        this.play(head);
        return true;
      }
    }
    if (this.concealing >= GIVE_UP) {
      this.stop();
      return false;
    }
    // Missing: silence for one frame, and keep waiting for it.
    this.stats.concealed++;
    this.concealing += this.frameLength;
    this.current = SILENCE.subarray(0, this.frameLength);
    this.read = 0;
    this.currentEnds = false;
    return true;
  }

  private play(frame: Frame): void {
    this.stats.played++;
    this.current = frame.pcm;
    this.read = 0;
    this.currentEnds = frame.end;
    this.cursor = frame.ts + frame.pcm.length;
    this.concealing = 0;
  }

  /** Ends the spurt; the next one starts at once if its frames wait. */
  private stop(): void {
    this.state = "idle";
    this.current = null;
    this.read = 0;
    this.currentEnds = false;
    this.concealing = 0;
    if (this.queue.length > 0) this.startSpurt();
  }

  /** A new talk spurt from the first frame waiting: the only moment the
      target may change. */
  private startSpurt(): void {
    const first = this.queue[0];
    this.target = this.targetDelay();
    this.state = "waiting";
    this.cursor = first.ts;
    this.playAt = first.ts + this.onTime() + this.target;
  }

  /** Inserts in timestamp order; a duplicate is dropped. */
  private insert(frame: Frame): void {
    let at = this.queue.length;
    while (at > 0 && this.queue[at - 1].ts > frame.ts) at--;
    if (at > 0 && this.queue[at - 1].ts === frame.ts) {
      this.stats.late++;
      return;
    }
    this.queue.splice(at, 0, frame);
  }

  /** Samples waiting: the queue plus what is left of the current frame. */
  private queued(): number {
    let total = this.current ? this.current.length - this.read : 0;
    for (const f of this.queue) total += f.pcm.length;
    return total;
  }

  private onTime(): number {
    let min = Infinity;
    for (let i = 0; i < this.transitCount; i++) min = Math.min(min, this.transits[i]);
    return min;
  }

  /** The 95th percentile of recent jitter plus the margin, at least one frame. */
  private targetDelay(): number {
    const n = this.transitCount;
    if (n === 0) return this.frameLength;
    const onTime = this.onTime();
    for (let i = 0; i < n; i++) this.scratch[i] = this.transits[i] - onTime;
    const jitter = this.scratch.subarray(0, n).sort();
    const p95 = jitter[Math.min(n - 1, Math.floor(n * 0.95))];
    return Math.max(this.frameLength, Math.ceil(p95) + MARGIN);
  }
}

/** Long enough for any frame Opus makes at 48 kHz (120 ms). */
const SILENCE = new Float32Array(5760);

/** The mixer's soft clip, as the core's: transparent up to 0.5, then
    bending towards ±1, so a loud sum saturates gently. */
export function softClip(x: number): number {
  const KNEE = 0.5;
  const m = Math.abs(x);
  if (m <= KNEE) return x;
  const bent = KNEE + (1 - KNEE) * Math.tanh((m - KNEE) / (1 - KNEE));
  return x < 0 ? -bent : bent;
}
