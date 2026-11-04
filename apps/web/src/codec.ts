// Opus through the browser's WebCodecs, for now (ADR 0040): 48 kHz, mono,
// raw Opus packets, in the low-delay mode where the browser honours it.
// It runs on the page's main thread: WebCodecs does not exist in an
// AudioWorklet.

export const SAMPLE_RATE = 48_000;

export type EncoderSettings = { frameMs: 2.5 | 5 | 10; bitrate: number };

/** The native clients' frames and bitrate (core/drywire-core, sender.rs),
    then a larger frame for an engine that refuses them. */
export const SETTINGS: readonly EncoderSettings[] = [
  { frameMs: 5, bitrate: 64_000 },
  { frameMs: 10, bitrate: 64_000 },
];

function opusConfig(s: EncoderSettings): AudioEncoderConfig {
  return {
    codec: "opus",
    sampleRate: SAMPLE_RATE,
    numberOfChannels: 1,
    bitrate: s.bitrate,
    opus: {
      format: "opus",
      frameDuration: s.frameMs * 1000,
      // CELT only, as the native clients (spec section 7). Only Chrome
      // honours it; Firefox and Safari encode in the default mode.
      application: "lowdelay",
      complexity: 9,
    },
  } as AudioEncoderConfig;
}

/** Whether this browser can encode Opus as the settings ask. */
export async function supported(s: EncoderSettings): Promise<boolean> {
  if (typeof AudioEncoder === "undefined" || typeof AudioDecoder === "undefined") return false;
  const r = await AudioEncoder.isConfigSupported(opusConfig(s)).catch(() => ({ supported: false }));
  return r.supported === true;
}

/** The first settings this browser can encode with, or null. */
export async function bestSettings(): Promise<EncoderSettings | null> {
  for (const s of SETTINGS) if (await supported(s)) return s;
  return null;
}

/** What waits for its frame to come out of a codec, by the frame's
    timestamp in microseconds. A codec keeps the timestamps it is given; if
    one ever shifts them, the oldest waiting frame is the one: Opus gives
    one output per input, in order. */
class Waiting<T> {
  private readonly byTime = new Map<number, T>();

  add(time: number, data: T): void {
    this.byTime.set(time, data);
  }

  take(time: number): T | undefined {
    const data = this.byTime.get(time);
    if (data !== undefined) {
      this.byTime.delete(time);
      return data;
    }
    for (const [oldest, d] of this.byTime) {
      this.byTime.delete(oldest);
      return d;
    }
    return undefined;
  }
}

const micros = (ts: number) => Math.round((ts * 1e6) / SAMPLE_RATE);

/** Encodes PCM frames; each Opus packet comes back with the data given
    for its frame. */
export class OpusEncoder<T> {
  private readonly waiting = new Waiting<T>();
  private readonly encoder: AudioEncoder;

  constructor(s: EncoderSettings, onPacket: (packet: Uint8Array, data: T) => void) {
    this.encoder = new AudioEncoder({
      output: (chunk) => {
        const packet = new Uint8Array(chunk.byteLength);
        chunk.copyTo(packet);
        const data = this.waiting.take(chunk.timestamp);
        if (data !== undefined) onPacket(packet, data);
      },
      error: (err) => console.error("opus encoder", err),
    });
    this.encoder.configure(opusConfig(s));
  }

  encode(pcm: Float32Array<ArrayBuffer>, ts: number, data: T): void {
    if (this.encoder.state !== "configured") return;
    this.waiting.add(micros(ts), data);
    this.encoder.encode(
      new AudioData({
        format: "f32-planar",
        sampleRate: SAMPLE_RATE,
        numberOfFrames: pcm.length,
        numberOfChannels: 1,
        timestamp: micros(ts),
        data: pcm,
      }),
    );
  }

  close(): void {
    if (this.encoder.state !== "closed") this.encoder.close();
  }
}

/** Decodes one sender's Opus packets, in the order given. */
export class OpusDecoder<T> {
  private readonly waiting = new Waiting<T>();
  private readonly decoder: AudioDecoder;

  constructor(onPcm: (pcm: Float32Array<ArrayBuffer>, data: T) => void) {
    this.decoder = new AudioDecoder({
      output: (frame) => {
        const pcm = new Float32Array(frame.numberOfFrames);
        frame.copyTo(pcm, { planeIndex: 0, format: "f32-planar" });
        const data = this.waiting.take(frame.timestamp);
        frame.close();
        if (data !== undefined) onPcm(pcm, data);
      },
      error: (err) => console.error("opus decoder", err),
    });
    this.decoder.configure({ codec: "opus", sampleRate: SAMPLE_RATE, numberOfChannels: 1 });
  }

  decode(packet: Uint8Array<ArrayBuffer>, ts: number, data: T): void {
    if (this.decoder.state !== "configured") return;
    this.waiting.add(micros(ts), data);
    this.decoder.decode(new EncodedAudioChunk({ type: "key", timestamp: micros(ts), data: packet }));
  }

  close(): void {
    if (this.decoder.state !== "closed") this.decoder.close();
  }
}
