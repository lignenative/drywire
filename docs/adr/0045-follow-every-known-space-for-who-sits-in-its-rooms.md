# 0045. Follow every known space's event stream for who sits in its rooms

Status: Accepted · 2026-11-04

## Context

The desktop app's side menu lists, under every room of every space it
knows, who sits there (the design canvas's V3 page), so a player sees
where their friends are before joining. The app knew it only for the
room of its own call: the session thread follows its space's event
stream, and nothing else did.

The control plane already says it. A space's event stream
(`GET /v1/spaces/{space}/events`, protocol section 13.6) begins with a
snapshot of everyone in all its rooms, then reports each arrival and
departure. The server holds at most 8 streams per identity, and limits
requests per address.

## Decision

- The app follows the stream of every space it lists, from the first
  listing until it quits, on a thread per space
  (`desktop/src-tauri/src/rooms.rs`). Who sits where is kept in memory
  only, like the server's own presence.
- At most 6 spaces per instance are followed: a call opens its own
  stream, and a reconnection may hold two for a moment, which keeps the
  app under the server's 8. Rooms of the spaces beyond show without
  names, as before.
- The call keeps its own stream. Sharing one would save a connection but
  tie the call's join, which must never miss an arrival, to a stream
  opened earlier for another purpose; a second idle connection per space
  in a call is cheap.
- At every change the app pushes the spaces, with who sits in each room,
  to the window as a `spaces` event, built from the last listing: no
  request to a server per change, and no polling.
- While a stream cannot be opened, its space's rooms show without names
  rather than stale ones, and it is tried again after 2 seconds, then
  up to every 30. A `404` (no longer a member, or the space was deleted)
  stops following it.
- The server and the protocol do not change.

## Consequences

- The side menu shows who sits in every room within a moment of them
  sitting down, in a call or not; the desktop test moves a person between
  rooms and watches another app see it.
- Each followed space holds an HTTP connection open, idle but for a
  comment every 15 seconds.
- A player with more than 6 spaces on one instance sees names in the
  first 6 listed only. If that becomes common, the server could offer one
  stream for all of an identity's spaces; that would be a protocol
  change.
- The web client can follow the same streams, under the same limit (a
  streamed `fetch`, since the token goes in a header).
