# Architecture decision records

Each file records one significant decision: its context, what we chose, and
what follows from it. Records are numbered in order and never rewritten once
accepted; a later record supersedes an earlier one instead.

A record is short and has four parts:

```markdown
# NNNN. Title in the imperative

Status: Accepted · YYYY-MM-DD

## Context
## Decision
## Consequences
```

| Number | Decision |
| --- | --- |
| [0001](0001-size-replies-to-their-requests.md) | Size every reply to its request |
| [0002](0002-restart-the-audio-sequence-per-key-epoch.md) | Restart the audio sequence at each key epoch |
| [0003](0003-generate-the-test-vectors-in-go.md) | Generate the shared test vectors in Go |
| [0004](0004-configure-phase-1-rooms-with-a-json-file.md) | Configure phase 1 rooms with a JSON file |
| [0005](0005-keep-the-relay-a-single-loop-around-a-pure-handler.md) | Keep the relay a single loop around a pure handler |
| [0006](0006-encrypt-audio-from-phase-1.md) | Encrypt audio from phase 1 |
| [0007](0007-send-sender-announcements-through-the-relay.md) | Send sender announcements through the relay |
| [0008](0008-use-rustcrypto-and-dalek-in-the-core.md) | Use RustCrypto and dalek for the core's cryptography |
| [0009](0009-bind-libopus-through-opusic-sys.md) | Bind libopus through opusic-sys |
| [0010](0010-schedule-talk-spurts-from-the-fastest-recent-transit.md) | Schedule talk spurts from the fastest recent transit |
| [0011](0011-wait-for-a-late-frame-instead-of-skipping-it.md) | Wait for a late frame instead of skipping it |
| [0012](0012-start-over-when-a-sender-timeline-jumps.md) | Start over when a sender's timeline jumps |
| [0013](0013-write-the-audio-backends-against-the-os-apis.md) | Write the audio backends against the OS APIs |
| [0014](0014-send-from-the-capture-callback.md) | Send from the capture callback |
| [0015](0015-keep-one-period-queued-for-playback.md) | Keep one period queued for playback |
| [0016](0016-never-wait-on-the-socket-in-the-capture-callback.md) | Never wait on the socket in the capture callback |
| [0017](0017-keep-the-audio-callbacks-free-of-allocation.md) | Keep the audio callbacks free of allocation |
| [0018](0018-resample-macos-microphones-in-the-backend.md) | Resample macOS microphones in the backend |
| [0019](0019-replay-a-deletion-ledger-at-every-start.md) | Replay a deletion ledger at every start |
| [0020](0020-log-in-with-a-challenge-signed-for-one-instance.md) | Log in with a challenge signed for one instance |
| [0021](0021-hand-out-one-session-per-identity.md) | Hand out one session per identity, and end it when the relay stops hearing it |
| [0022](0022-talk-to-the-control-plane-with-ureq-and-rustls.md) | Talk to the control plane with ureq and rustls |
| [0023](0023-hand-playback-a-ready-track-for-a-new-member.md) | Hand playback a ready track for a member who joins mid-call |
| [0024](0024-replace-the-phase-1-files-with-the-control-plane.md) | Replace the phase 1 files with the control plane |
| [0025](0025-give-the-server-binary-its-own-maintenance-commands.md) | Give the server binary its own maintenance commands |
| [0026](0026-run-the-server-in-a-scratch-image-on-the-host-network.md) | Run the server in a scratch image on the host network |
| [0027](0027-let-a-proxy-on-a-private-network-reach-the-control-plane.md) | Let a proxy on a private network reach the control plane |
