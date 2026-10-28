// The three presence sounds (spec section 10): someone joined, left, or
// lost their connection. Short, distinct and quiet, synthesised with Web
// Audio, so there is no sound file to license. They play through the web
// view's own audio output, never through the call: they cannot delay a
// teammate's voice.
import type { Notice } from "./api";

/** One note: a frequency in hertz, a start and a length in seconds. */
export interface Note {
  hz: number;
  at: number;
  length: number;
}

/** The notes of each sound: rising for a join, falling for a leave, a
    low double knock for a lost connection. */
export function recipe(kind: Notice["kind"]): Note[] {
  switch (kind) {
    case "join":
      return [
        { hz: 660, at: 0, length: 0.07 },
        { hz: 880, at: 0.08, length: 0.1 },
      ];
    case "leave":
      return [
        { hz: 880, at: 0, length: 0.07 },
        { hz: 660, at: 0.08, length: 0.1 },
      ];
    case "disconnect":
      return [
        { hz: 330, at: 0, length: 0.06 },
        { hz: 330, at: 0.1, length: 0.06 },
      ];
  }
}

/** The loudest a presence sound gets, at volume 1: mixed low. */
const PEAK = 0.25;

let context: AudioContext | null = null;

/** Plays the sound of `kind` at `volume`, 0 (off) to 1. */
export function playPresence(kind: Notice["kind"], volume: number): void {
  if (volume <= 0) return;
  context ??= new AudioContext({ latencyHint: "interactive" });
  const start = context.currentTime + 0.01;
  for (const note of recipe(kind)) {
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    oscillator.type = "sine";
    oscillator.frequency.value = note.hz;
    // A short fade in and out, so no note clicks.
    const t0 = start + note.at;
    const t1 = t0 + note.length;
    gain.gain.setValueAtTime(0, t0);
    gain.gain.linearRampToValueAtTime(PEAK * volume, t0 + 0.005);
    gain.gain.setValueAtTime(PEAK * volume, t1 - 0.02);
    gain.gain.linearRampToValueAtTime(0, t1);
    oscillator.connect(gain).connect(context.destination);
    oscillator.start(t0);
    oscillator.stop(t1);
  }
}
