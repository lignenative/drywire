// The room's side of the call, on the audio thread: one jitter buffer per
// person (jitter.ts), each at its own volume, summed and soft-clipped
// into the output (spec section 7, "Reception"). Decoded frames come from
// the page, which decodes Opus with WebCodecs (ADR 0040).
import { JitterBuffer, softClip } from "./jitter";
import type { FromPlayback, SenderStats, ToPlayback } from "./messages";

/** Stats ten times a second: what the interface polls. */
const STATS_EVERY = 4800;

class Playback extends AudioWorkletProcessor {
  private readonly buffers = new Map<number, JitterBuffer>();
  private readonly gains = new Map<number, number>();
  private sinceStats = 0;

  constructor() {
    super();
    this.port.onmessage = (e: MessageEvent<ToPlayback>) => {
      const m = e.data;
      if (m.type === "frame") {
        let buffer = this.buffers.get(m.session);
        if (!buffer) this.buffers.set(m.session, (buffer = new JitterBuffer()));
        buffer.push({ ts: m.ts, pcm: m.pcm, end: m.end }, currentFrame);
      } else if (m.type === "volume") {
        this.gains.set(m.session, m.gain);
      } else if (m.type === "forget") {
        this.buffers.delete(m.session);
        this.gains.delete(m.session);
      }
    };
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const channels = outputs[0];
    const out = channels[0];
    out.fill(0);
    for (const [session, buffer] of this.buffers) {
      buffer.pull(out, 0, out.length, currentFrame, this.gains.get(session) ?? 1);
    }
    for (let i = 0; i < out.length; i++) out[i] = softClip(out[i]);
    for (let c = 1; c < channels.length; c++) channels[c].set(out);

    this.sinceStats += out.length;
    if (this.sinceStats >= STATS_EVERY) {
      this.sinceStats = 0;
      const senders: SenderStats[] = [];
      for (const [session, b] of this.buffers) {
        const s = b.stats;
        senders.push({
          session,
          received: s.received,
          played: s.played,
          concealed: s.concealed,
          late: s.late,
          dropped: s.dropped,
          depth: s.depth,
          targetMs: (s.targetSamples * 1000) / sampleRate,
          active: s.active,
        });
      }
      const message: FromPlayback = { type: "stats", senders };
      this.port.postMessage(message);
    }
    return true;
  }
}

registerProcessor("drywire-playback", Playback);
