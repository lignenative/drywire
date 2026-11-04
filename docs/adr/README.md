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
| [0028](0028-serve-the-relay-on-udp-30050-in-production.md) | Serve the relay on UDP 30050 in production |
| [0029](0029-name-the-command-line-client-drywire.md) | Name the command-line client `drywire` |
| [0030](0030-build-the-desktop-app-in-its-own-cargo-workspace.md) | Build the desktop app in its own Cargo workspace |
| [0031](0031-protect-the-desktop-identity-with-dpapi.md) | Protect the desktop app's identity with DPAPI |
| [0032](0032-control-a-running-call-without-touching-the-audio-path.md) | Control a running call without touching the audio path |
| [0033](0033-play-presence-sounds-outside-the-call.md) | Play the presence sounds outside the call |
| [0034](0034-ask-for-gate-calibration-and-lean-the-default-towards-sending.md) | Ask for gate calibration, and lean the default towards sending |
| [0035](0035-join-again-when-the-server-forgets-the-session.md) | Join again when the server forgets the session |
| [0036](0036-keep-the-access-log-in-daily-files-the-server-prunes.md) | Keep the access log in daily files the server prunes |
| [0037](0037-build-the-interface-from-shadcn-components-on-base-ui.md) | Build the interface from shadcn components on Base UI |
| [0038](0038-reach-the-relay-from-browsers-over-webtransport.md) | Reach the relay from browsers over WebTransport, on a port of its own |
| [0039](0039-pin-the-web-relay-certificate-by-its-hash.md) | Pin the web relay's certificate by its hash |
| [0040](0040-run-the-protocol-in-the-browser-as-webassembly.md) | Run the protocol in the browser as WebAssembly, and Opus through WebCodecs for now |
| [0041](0041-close-the-lock-only-when-everyone-heard-is-verified.md) | Close the lock only when everyone heard is verified |
| [0043](0043-keep-the-website-and-the-hosted-offer-out-of-this-repository.md) | Keep the website and the hosted offer out of this repository |
| [0044](0044-open-the-microphone-for-the-settings-meter-only-while-it-shows.md) | Open the microphone for the settings' meter only while it shows |
| [0045](0045-follow-every-known-space-for-who-sits-in-its-rooms.md) | Follow every known space's event stream for who sits in its rooms |
| [0046](0046-follow-the-system-default-device-during-a-call.md) | Follow the system's default device during a call (proposed) |
| [0047](0047-end-the-oldest-event-stream-rather-than-refuse-a-new-one.md) | End the oldest event stream rather than refuse a new one |
| [0048](0048-share-the-desktop-interface-with-the-web-client.md) | Share the desktop interface with the web client |
| [0049](0049-serve-the-web-client-from-each-instance-beside-its-api.md) | Serve the web client from each instance, beside its API |
| [0050](0050-let-the-person-start-the-next-reconnection-attempt.md) | Let the person start the next reconnection attempt |
| [0051](0051-make-the-identity-an-account-that-follows-its-person.md) | Make the identity an account that follows its person (to be replaced by 0055) |
| [0052](0052-take-space-keys-out-of-invitation-links-with-mls.md) | Take space keys out of invitation links with MLS (accepted, steps 1 and 2 of 5 built, step 3 in the core) |
| [0053](0053-compare-a-safety-code-to-check-who-a-key-belongs-to.md) | Compare a safety code to check who a key belongs to |
| [0054](0054-let-the-operator-hand-out-codes-that-create-a-space.md) | Let the operator hand out codes that create a space (to be replaced by 0055) |
| [0055](0055-sign-in-with-an-email-code-and-a-unique-user-name.md) | Sign in with an e-mail code and a unique user name (accepted, not built) |
