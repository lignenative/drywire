# 0013. Write the audio backends against the OS APIs

Status: Accepted · 2026-10-27

## Context

The spec (section 6) puts audio I/O behind one small trait with one backend
per OS, written directly against the OS API, because generic audio
libraries do not expose the lowest-latency modes: WASAPI exclusive mode and
`IAudioClient3` periods on Windows, the HAL with its smallest I/O buffer on
macOS. The question left is how to call those APIs from Rust.

- **macOS, Core Audio.** A plain C API: a dozen functions, a few
  structures, integer constants. The ready-made bindings are either
  generated at build time with `bindgen` (`coreaudio-sys`, which needs
  libclang on every machine that builds) or a large generated crate family
  (`objc2-core-audio` and its dependencies).
- **Windows, WASAPI.** A COM API: interfaces are tables of function
  pointers, and a binding that gets one slot wrong in a vtable calls the
  wrong function with no error. Microsoft publishes the official Rust
  bindings, the `windows` crate (MIT or Apache 2.0), generated from the
  Windows metadata.

We cannot run the Windows backend on the development Mac, only compile it.

## Decision

- **Core Audio is bound by hand**, in `drywire-core/src/audio/coreaudio.rs`:
  only what the backend calls, with Apple's names, so each declaration can
  be checked against the headers. No build-time code generation, no new
  dependency.
- **WASAPI goes through the `windows` crate**, as a dependency for Windows
  targets only, with just the features the backend uses. Hand-written COM
  vtables in code we cannot run here would be a source of silent memory
  corruption; generated official bindings remove that risk.
- Both backends speak the same format to the rest of the core: mono 32-bit
  float at 48 kHz, converted by the backend.
- Neither enables voice processing: on macOS the HAL output unit, never the
  voice processing unit; on Windows a raw stream with no audio effects.

## Consequences

- macOS builds need nothing beyond Rust, CMake (for libopus) and Xcode's
  command line tools.
- The `windows` crate only compiles for Windows targets; macOS and Linux
  builds do not build it. It brings `windows-core`, `windows-link` and a
  few small helpers of the same project, all MIT or Apache 2.0.
- A mistake in the hand-written Core Audio declarations shows up as an
  error status or a crash on the first call, which `drywire-cli devices`
  exercises without opening the microphone.
