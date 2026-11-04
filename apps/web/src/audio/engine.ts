// The browser's sound card for a call: the microphone, untouched, into
// the capture worklet; the playback worklet into the speakers; Opus in
// between, through WebCodecs on this thread (ADR 0040).
//
// Latency first. The context runs at 48 kHz, Opus's own rate, so nothing
// resamples when the device runs at 48 kHz too. Its latency hint is 0: the
// smallest output buffer the browser allows, one render quantum (128
// samples, 2.7 ms) in Chrome on a Mac, where "interactive" gave twice
// that. Sixty seconds of a call with the echo bot had no underrun either
// way (apps/web/README.md, "Measure"); a machine that glitches at 0 would
// call for "interactive". The capture asks for no echo cancellation, no
// noise suppression and no automatic gain, which is the untouched voice
// the spec wants and also what lets Chrome use the device's native buffer
// instead of forcing 10 ms ones. The worklets add no buffer of their own
// beyond one Opus frame on the way out and the jitter buffer on the way
// in.
import type { DeviceView } from "@/api";
import { SAMPLE_RATE } from "../codec";
import captureUrl from "./capture.worklet.ts?worker&url";
import type { FromCapture, FromPlayback, ToCapture, ToPlayback } from "./messages";
import playbackUrl from "./playback.worklet.ts?worker&url";

/** Raised when the browser refuses the microphone. Its message is empty:
    the interface's sentence says it all ("error.microphone"). */
export class MicrophoneDenied extends Error {}

/** Whether the microphone is being asked for, refused, or neither: the
    web client's dialog follows it (MicDialog.tsx). */
export type MicState = "idle" | "asking" | "denied";

const micListeners = new Set<(state: MicState) => void>();
let micState: MicState = "idle";

export function onMicState(f: (state: MicState) => void): () => void {
  micListeners.add(f);
  f(micState);
  return () => micListeners.delete(f);
}

export function setMicState(state: MicState): void {
  micState = state;
  for (const f of micListeners) f(state);
}

/** The microphone, as the spec wants it: nothing in the browser touches
    the voice (spec section 10, "Web"). */
export async function openMicrophone(): Promise<MediaStream> {
  const permission = await navigator.permissions
    ?.query({ name: "microphone" as PermissionName })
    .then((p) => p.state)
    .catch(() => "prompt" as const);
  if (permission === "denied") {
    setMicState("denied");
    throw new MicrophoneDenied("");
  }
  if (permission !== "granted") setMicState("asking");
  try {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
        channelCount: 1,
      },
    });
    setMicState("idle");
    return stream;
  } catch (e) {
    if (e instanceof DOMException && (e.name === "NotAllowedError" || e.name === "SecurityError")) {
      setMicState("denied");
      throw new MicrophoneDenied("");
    }
    setMicState("idle");
    throw e;
  }
}

export interface PlaybackStats {
  seconds: number;
  underruns: number;
  underrunMs: number;
  baseLatencyMs: number;
  outputLatencyMs: number;
}

export class AudioEngine {
  private captureListeners = new Set<(m: FromCapture) => void>();
  private statsListeners = new Set<(m: FromPlayback) => void>();
  private closeListeners = new Set<() => void>();
  private closed = false;

  private constructor(
    readonly context: AudioContext,
    private readonly stream: MediaStream,
    private readonly source: MediaStreamAudioSourceNode,
    private readonly capture: AudioWorkletNode,
    private readonly playback: AudioWorkletNode,
    private readonly outputName: string,
  ) {
    capture.port.onmessage = (e: MessageEvent<FromCapture>) => {
      for (const f of this.captureListeners) f(e.data);
    };
    playback.port.onmessage = (e: MessageEvent<FromPlayback>) => {
      for (const f of this.statsListeners) f(e.data);
    };
  }

  /** Opens the microphone and the speakers, with frames of `frameSamples`. */
  static async start(frameSamples: number): Promise<AudioEngine> {
    const stream = await openMicrophone();
    const context = new AudioContext({ sampleRate: SAMPLE_RATE, latencyHint: 0 });
    try {
      await Promise.all([context.audioWorklet.addModule(captureUrl), context.audioWorklet.addModule(playbackUrl)]);
      const source = context.createMediaStreamSource(stream);
      const capture = new AudioWorkletNode(context, "drywire-capture", {
        numberOfInputs: 1,
        numberOfOutputs: 0,
        channelCount: 1,
        channelCountMode: "explicit",
        processorOptions: { frameSamples },
      });
      const playback = new AudioWorkletNode(context, "drywire-playback", {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [1],
      });
      source.connect(capture);
      playback.connect(context.destination);
      // Joining is a click: the context may start, but some engines want
      // to be told.
      if (context.state !== "running") await context.resume();
      // The browser plays to the system's default output; its name is
      // readable once the microphone is allowed.
      const outputs = await navigator.mediaDevices.enumerateDevices().catch(() => []);
      const speakers = outputs.filter((d) => d.kind === "audiooutput");
      const output = speakers.find((d) => d.deviceId === "default") ?? speakers[0];
      return new AudioEngine(context, stream, source, capture, playback, output?.label || "");
    } catch (e) {
      stream.getTracks().forEach((t) => t.stop());
      await context.close().catch(() => {});
      throw e;
    }
  }

  onCapture(f: (m: FromCapture) => void): () => void {
    this.captureListeners.add(f);
    return () => this.captureListeners.delete(f);
  }

  onStats(f: (m: FromPlayback) => void): () => void {
    this.statsListeners.add(f);
    return () => this.statsListeners.delete(f);
  }

  /** Calls `f` once the engine closes; at once if it already has. */
  onClose(f: () => void): () => void {
    if (this.closed) {
      f();
      return () => {};
    }
    this.closeListeners.add(f);
    return () => this.closeListeners.delete(f);
  }

  toCapture(m: ToCapture): void {
    this.capture.port.postMessage(m);
  }

  toPlayback(m: ToPlayback): void {
    this.playback.port.postMessage(m, m.type === "frame" ? [m.pcm.buffer] : []);
  }

  /** What the browser says of its two streams, in milliseconds: reported,
      never measured (the loopback test needs the app). */
  devices(): DeviceView[] {
    const track = this.stream.getAudioTracks()[0];
    const settings = (track?.getSettings() ?? {}) as MediaTrackSettings & { latency?: number };
    const name = track?.label || "Microphone";
    const quantumMs = (128 * 1000) / SAMPLE_RATE;
    return [
      {
        direction: "input",
        name,
        mode: "Web Audio",
        bufferMs: quantumMs,
        latencyMs: (settings.latency ?? 0) * 1000,
        bluetooth: /bluetooth|airpods|wireless|hands-free/i.test(name),
      },
      {
        direction: "output",
        name: this.outputName,
        mode: "Web Audio",
        bufferMs: this.context.baseLatency * 1000,
        latencyMs: (this.context.outputLatency || 0) * 1000,
        bluetooth: /bluetooth|airpods|wireless|hands-free/i.test(this.outputName),
      },
    ];
  }

  /** What the browser counted of the output since the call started, where
      it counts it (Chrome's AudioContext.playbackStats): a glitch is an
      underrun. Null elsewhere. */
  playbackStats(): PlaybackStats | null {
    const stats = (this.context as AudioContext & { playbackStats?: Record<string, number> }).playbackStats;
    if (!stats) return null;
    return {
      seconds: stats.totalDuration,
      underruns: stats.underrunEvents,
      underrunMs: stats.underrunDuration * 1000,
      baseLatencyMs: this.context.baseLatency * 1000,
      outputLatencyMs: (this.context.outputLatency || 0) * 1000,
    };
  }

  /** Input and output, as the browser reports them. */
  deviceMs(): number {
    return this.devices().reduce((sum, d) => sum + d.bufferMs + d.latencyMs, 0);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const f of this.closeListeners) f();
    this.closeListeners.clear();
    this.source.disconnect();
    this.capture.disconnect();
    this.playback.disconnect();
    this.stream.getTracks().forEach((t) => t.stop());
    this.context.close().catch(() => {});
  }
}
