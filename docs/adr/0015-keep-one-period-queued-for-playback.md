# 0015. Keep one period queued for playback

Status: Accepted · 2026-10-27

## Context

A shared-mode WASAPI stream has a buffer several engine periods long. The
audio engine takes one period from it per pass and, in event mode, wakes
the client right after. The first version of the Windows backend wrote as
many frames as were free on each wake-up, which is what Microsoft's basic
samples do. After the first wake-up the buffer is full, and it stays full:
each wake-up refills the period the engine just took. Every queued frame
is played in order, so the whole buffer length sits in the mouth-to-ear
path, on top of the engine's own latency.

Core Audio has no such queue: the HAL asks for exactly one I/O buffer per
callback.

## Decision

- The render loop keeps **at most one engine period queued**. On each
  wake-up it writes `period − padding` frames, which is one period when
  the engine has just drained the queue, and nothing if it has not.
- One period of silence is written before the stream starts, so the
  engine's first pass finds something to play.
- The period is the one the stream was opened with: the smallest
  `IAudioClient3` period at 48 kHz, or the engine's default period when
  Windows resamples.
- Capture is unchanged: exclusive mode where possible, where the buffer is
  one period by construction, and every packet is handed on as soon as the
  engine signals it.

## Consequences

- Playback adds one period of queue instead of a whole buffer: at the
  smallest `IAudioClient3` period (often 128 to 144 frames, 2.7 to 3 ms)
  instead of several times that. Not measured yet on a Windows PC; the
  physical method of the spec will say.
- A wake-up later than one period lets the engine find the queue empty: it
  plays silence for that pass, and the queue starts again from one period.
  A late thread costs a click, never lasting latency. The thread runs in
  the "Pro Audio" class to make this rare. If physical tests show clicks,
  the target can become two periods; that would be a measured trade, made
  here.
