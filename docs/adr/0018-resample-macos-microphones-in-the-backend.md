# 0018. Resample macOS microphones in the backend

Status: Accepted · 2026-10-27

## Context

Drywire captures at 48 kHz, Opus's native rate. On macOS the HAL output
unit converts formats and channels for us, but never the sample rate of
an input (ADR 0013). Until now a microphone at another rate was refused,
with a message pointing to Audio MIDI Setup. Many USB microphones and
headsets run at 44.1 kHz by default, and some at 32 or 16 kHz, so the
first thing many people would have seen is an error.

Converting the rate costs latency: any good low-pass filter has to see
samples after the one it outputs. The choice is how much.

- **Linear interpolation** adds nothing, but dulls the top of the voice
  band and aliases audibly.
- **A polyphase windowed-sinc filter** of 32 taps waits for 16 input
  samples, 0.36 ms at 44.1 kHz: in the measured response it is flat
  within 0.01 dB to 17 kHz, keeps 62 to 68 dB of signal-to-error ratio on
  tones, and puts aliases 100 dB down when the rate goes down.
- **Longer filters** would add delay for a difference nobody hears in a
  voice.

## Decision

- `drywire-core/src/audio/resample.rs` converts any ratio a real device
  has (reduced to `up / down`, at most 1024 phases) with precomputed
  windowed-sinc filters: 32 taps, Kaiser window for 60 dB, cut half a
  transition band below the lower Nyquist frequency, a gain of exactly 1
  per phase. Going down, the filters get proportionally more taps.
- The Core Audio backend captures a microphone not at 48 kHz at its own
  rate and converts in the input callback, with buffers made before the
  stream starts (ADR 0017).
- The delay is reported: the stream's mode says it resamples, and the
  reported latency gets a `resampler` part.
- A rate no ratio covers is still refused, with the same advice.

## Consequences

- A 44.1 kHz microphone works, for 0.36 ms more latency than the same
  microphone at 48 kHz. Setting it to 48 kHz stays the lowest-latency
  setting, and `drywire-cli devices` shows the difference.
- Tested by unit tests only; the development Mac's microphones all run
  at 48 kHz. To check on a USB microphone at 44.1 kHz.
- Windows needs none of it: WASAPI shared mode resamples in the engine,
  and exclusive mode only opens 48 kHz formats.
