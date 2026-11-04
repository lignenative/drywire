// Opus through the browser's WebCodecs, for now (ADR 0040): 48 kHz, mono,
// raw Opus packets, in the low-delay mode where the browser honours it.

export const SAMPLE_RATE = 48_000;

export type EncoderSettings = { frameMs: 2.5 | 5 | 10; bitrate: number };

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
  const r = await AudioEncoder.isConfigSupported(opusConfig(s));
  return r.supported === true;
}

/** Encodes PCM frames; each Opus packet goes to onFrame. */
export function encoder(s: EncoderSettings, onFrame: (frame: Uint8Array) => void): AudioEncoder {
  const e = new AudioEncoder({
    output: (chunk) => {
      const frame = new Uint8Array(chunk.byteLength);
      chunk.copyTo(frame);
      onFrame(frame);
    },
    error: (err) => console.error("opus encoder", err),
  });
  e.configure(opusConfig(s));
  return e;
}

/** Decodes Opus packets; each block of samples goes to onPcm. */
export function decoder(onPcm: (pcm: Float32Array) => void): AudioDecoder {
  const d = new AudioDecoder({
    output: (data) => {
      const pcm = new Float32Array(data.numberOfFrames);
      data.copyTo(pcm, { planeIndex: 0, format: "f32-planar" });
      data.close();
      onPcm(pcm);
    },
    error: (err) => console.error("opus decoder", err),
  });
  d.configure({ codec: "opus", sampleRate: SAMPLE_RATE, numberOfChannels: 1 });
  return d;
}
