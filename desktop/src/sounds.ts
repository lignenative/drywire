// The three presence sounds (spec section 10): someone joined, left, or
// lost their connection. Short, distinct and quiet, synthesised with Web
// Audio, so there is no sound file to license. They play through the web
// view's own audio output, never through the call: they cannot delay a
// teammate's voice.
//
// Each note is a small wooden bar struck with a mallet. Its partials are
// a marimba's (1, 3.93 and 9.2 times the pitch, the higher ones dying
// first), with a quiet octave below for warmth; a 6 ms burst of filtered
// noise is the mallet's knock; the pitch slides into place over 25 ms,
// up for a join and down for a leave. A low-pass filter rounds the whole,
// and one echo 80 ms later gives it a little room. The sounds were chosen
// by ear from rendered candidates; this file reproduces the chosen one.
import type { Notice } from "./api";

/** One note: a frequency in hertz, when it starts and how long it rings,
    in seconds, and its loudness, 0 to 1. */
export interface Note {
  hz: number;
  at: number;
  length: number;
  gain: number;
}

/** A sound: its notes, and how they are played. */
export interface Recipe {
  notes: Note[];
  /** How far below its pitch each note starts, as a fraction (negative:
      above), sliding to it in the first 25 ms. */
  glide: number;
  /** The low-pass filter's cutoff, in hertz. */
  cutoff: number;
  /** Brings the three sounds to the same loudness. */
  level: number;
}

/** The partials of a note, as multiples of its pitch, their loudness, and
    how much faster than the fundamental they fade. */
const PARTIALS = [
  { ratio: 1, gain: 1, decay: 1 },
  { ratio: 3.93, gain: 0.22, decay: 3.5 },
  { ratio: 9.2, gain: 0.05, decay: 7 },
  { ratio: 0.5, gain: 0.12, decay: 0.8 },
];

const C4 = 261.63;
const C5 = 523.25;
const G5 = 783.99;

/** A join rises a fifth, C5 to G5; a leave falls it; a lost connection is
    two muted knocks on a low C. */
export function recipe(kind: Notice["kind"]): Recipe {
  switch (kind) {
    case "join":
      return {
        notes: [
          { hz: C5, at: 0, length: 0.14, gain: 0.9 },
          { hz: G5, at: 0.07, length: 0.2, gain: 1 },
        ],
        glide: 0.03,
        cutoff: 4000,
        level: 1,
      };
    case "leave":
      return {
        notes: [
          { hz: G5, at: 0, length: 0.14, gain: 0.85 },
          { hz: C5, at: 0.07, length: 0.2, gain: 0.9 },
        ],
        glide: -0.03,
        cutoff: 4000,
        level: 0.86,
      };
    case "disconnect":
      return {
        notes: [
          { hz: C4, at: 0, length: 0.1, gain: 1 },
          { hz: C4, at: 0.12, length: 0.12, gain: 0.8 },
        ],
        glide: 0,
        cutoff: 1200,
        level: 1.1,
      };
  }
}

/** The loudest a presence sound gets, at volume 1: about −12 dBFS. */
const PEAK = 0.22;
const ATTACK = 0.003;
const GLIDE_TIME = 0.025;
const MALLET = 0.3;
const MALLET_LENGTH = 0.006;
const ECHO_DELAY = 0.08;
const ECHO_GAIN = 0.22;

/** Web Audio gives a low-pass filter's Q in decibels. */
const qInDb = (q: number) => 20 * Math.log10(q);

let context: AudioContext | null = null;
let knock: AudioBuffer | null = null;

/** The mallet's knock: noise that dies away in about a millisecond. The
    same noise every time, so every knock sounds alike. */
function malletBuffer(ctx: AudioContext): AudioBuffer {
  const length = Math.round(MALLET_LENGTH * ctx.sampleRate);
  const buffer = ctx.createBuffer(1, length, ctx.sampleRate);
  const data = buffer.getChannelData(0);
  let seed = 7;
  for (let i = 0; i < length; i++) {
    seed = (seed * 1664525 + 1013904223) >>> 0;
    const noise = (seed / 0xffffffff) * 2 - 1;
    data[i] = noise * Math.exp(-i / ctx.sampleRate / 0.0012);
  }
  return buffer;
}

/** Plays the sound of `kind` at `volume`, 0 (off) to 1. */
export function playPresence(kind: Notice["kind"], volume: number): void {
  if (volume <= 0) return;
  context ??= new AudioContext({ latencyHint: "interactive" });
  const ctx = context;
  knock ??= malletBuffer(ctx);
  const sound = recipe(kind);
  const start = ctx.currentTime + 0.01;

  const filter = ctx.createBiquadFilter();
  filter.type = "lowpass";
  filter.frequency.value = sound.cutoff;
  filter.Q.value = qInDb(0.5);
  const echo = ctx.createDelay(0.2);
  echo.delayTime.value = ECHO_DELAY;
  const echoLevel = ctx.createGain();
  echoLevel.gain.value = ECHO_GAIN;
  filter.connect(ctx.destination);
  filter.connect(echo).connect(echoLevel).connect(ctx.destination);

  const loudness = PEAK * volume * sound.level;
  for (const note of sound.notes) {
    const t0 = start + note.at;
    for (const p of PARTIALS) {
      const oscillator = ctx.createOscillator();
      const gain = ctx.createGain();
      oscillator.type = "sine";
      const hz = note.hz * p.ratio;
      oscillator.frequency.setValueAtTime(hz * (1 - sound.glide), t0);
      oscillator.frequency.linearRampToValueAtTime(hz, t0 + GLIDE_TIME);
      // Up in 3 ms, so it does not click, then an exponential ring-out
      // that is all but silent at the note's end (sooner for the higher
      // partials).
      gain.gain.setValueAtTime(0, t0);
      gain.gain.linearRampToValueAtTime(loudness * note.gain * p.gain, t0 + ATTACK);
      gain.gain.setTargetAtTime(0, t0 + ATTACK, note.length / p.decay / 5);
      oscillator.connect(gain).connect(filter);
      oscillator.start(t0);
      oscillator.stop(t0 + note.length + 0.06);
    }

    const mallet = ctx.createBufferSource();
    mallet.buffer = knock;
    const tone = ctx.createBiquadFilter();
    tone.type = "lowpass";
    tone.frequency.value = Math.min(note.hz * 4, 6000);
    tone.Q.value = qInDb(0.7);
    const malletGain = ctx.createGain();
    malletGain.gain.value = loudness * note.gain * MALLET;
    mallet.connect(tone).connect(malletGain).connect(filter);
    mallet.start(t0);
  }
}
