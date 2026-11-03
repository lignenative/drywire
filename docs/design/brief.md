# Design brief: the Drywire desktop client

This brief is for whoever redesigns the desktop app's interface: a designer,
or a design tool given this file and the screenshots next to it. It says
what the app is, what every screen must be able to show, and what must not
change. It was written from the code at the time of writing (commit after
`57e78cc`); where they disagree, the code wins.

The screenshots in [`screens/`](screens/) are the current interface,
rendered at 2× with made-up data (generic names, `example.org` hosts).
They show what exists, not what it should look like.

## What Drywire is

Low-latency voice chat for a small group of friends who play games
together. Open source (MIT), self-hosted: a group runs its own server.
Desktop first (Windows now, macOS next, then iOS, Android, Linux, web).

What sets it apart, and what the interface must never contradict:

- **Latency first.** Every millisecond between a mouth and an ear counts.
  The app shows latency figures, and they must stay honest.
- **Measured, never promised.** A figure is shown either as a measurement
  or labelled as an estimate (today: "≈ 47 ms (estimate)"). The design must
  keep that distinction visible. No marketing number anywhere.
- **Untouched voice.** No noise suppression, no echo cancellation, no
  automatic gain. A *voice gate* only decides when to send: below a
  threshold the microphone sends nothing.
- **Out of the way during a match.** The window sits beside a game or is
  hidden in the tray; an overlay shows who speaks, over the game, without
  ever taking a click or the keyboard.

Not in scope, ever: text chat, video, screen sharing, recording, bots.
Don't design for them.

## The person using it

A player in a group of three to ten friends, with a headset, often a
controller in hand, a game in the foreground. They open Drywire, join a
room, and forget it is there until someone's voice cuts out or they need
to mute. They are not audio engineers: "voice gate" and "jitter buffer"
mean nothing to them unless explained in place.

## Direction from the project owner

- Very modern, the polish of the best consumer apps (Discord is the
  reference people will compare it to).
- **Very simple: only the essentials on screen.** Settings, statistics,
  device details and the latency lab are hidden until asked for.
- The presence sounds already chosen set the tone: soft, warm, wooden
  (`desktop/src/sounds.ts`).

## Constraints

- **Window**: 380 × 640 by default, at least 320 × 420, resizable. Often
  placed next to a game, so the call view must work narrow.
- **Overlay**: a separate transparent window, 260 × 220, top-left corner
  of the screen, always on top, lets every click through. It cannot take
  input. It works over games in borderless window mode, not exclusive
  fullscreen.
- **Tray icon** with a menu: Show Drywire, Mute/Unmute, Quit. Closing the
  window hides it to the tray; the call goes on.
- **Global shortcut** for mute (default Ctrl+Shift+M), works while a game
  has the focus.
- **Two languages**, English and French, switchable in the app. French
  strings run 15 to 30 % longer: leave room.
- **Dark first** (a game is usually dark around it). A light theme is
  welcome but optional.
- **Fonts**: today the system UI font (Segoe UI Variable on Windows).
  Any font must be free to ship under an open source licence (OFL, MIT,
  Apache).
- **Icons and illustrations** must be original or under a permissive
  licence. No brand logos.
- **Accessibility**: readable contrast, keyboard reachable, the speaking
  state not shown by colour alone.
- **Brand**: name "Drywire", tagline today "Voice for teams who play
  together". The logo is a placeholder (a green bar in a square):
  a real one is welcome.

## Screens and states

Every state below exists in the code. A design that leaves one out leaves
a hole in the app.

### 1. Home, out of a call

Today one long scrolling page of cards: spaces, invitation, device key,
latency lab, settings (screens 01, 02, 10).

| Element | What it shows or does | Today |
| --- | --- | --- |
| Spaces | Each space this device belongs to: its name, the server's host, its rooms; a Join button per room. A refresh. | Card "Spaces" |
| Space unreachable | A space whose server cannot be reached: an error under its name, no rooms | screen 10 |
| No space yet | Empty state: "No space yet. Paste an invitation link below." | — |
| Join with an invitation | Paste a link, type a display name, "Join the space". Success: "You are now a member of {space}." Errors: not a Drywire link, server refused, server unreachable. | Card |
| Device key | A 64-character hex key with Copy. Only server operators need it, to let someone create spaces. Rarely used. | Card "This device" |
| Calibrate first | Until the person has set their voice gate: a banner asking for the ten-second calibration, with a button to it. | screen 01 |
| Language | EN / FR switch | Header |

### 2. In a call: the widget

The heart of the app (screens 03 to 08, 12). Shown instead of the home
view while in a room.

| Element | What it shows or does |
| --- | --- |
| Where | Room name (`# general`), space name, server host |
| Link quality | Round trip to the relay, in ms, with a good / fair / poor colour (poor above 150 ms or 5 % loss, fair above 60 ms or 1 %). Hover: the uplink loss when there is some. |
| Microphone level | A live meter (–90 to 0 dBFS) with the gate's threshold drawn on it; the level shows "open" (sending) when above the threshold |
| Mute | A toggle; while muted, "Muted" is shown; the meter still moves (the person can check their mic) but nothing is sent |
| Voice gate threshold | A slider, –80 to –10 dB. Help text: "Your voice is sent only above this level. Set it just above your background noise." |
| Default threshold | Under the slider, until calibrated: "Default threshold: calibrate it in the latency lab, out of a call." (screen 07) |
| People | Everyone in the room: a speaking light, the name, "(you)" for this device. For the others: packet loss, an **estimated** mouth-to-ear latency ("≈ 47 ms (estimate)", with a tooltip explaining what it adds up), and a volume slider 0 to 200 %. |
| Statistics (hidden) | Round trip and its lowest; this device's input + output latency, marked "measured" or "reported by the system"; uplink loss; per person: received, played, concealed, late, dropped, buffered |
| Audio devices (hidden) | Input and output: device name, mode (WASAPI exclusive, shared…), buffer and reported latency |
| Bluetooth warning | When either device is Bluetooth: "Bluetooth: expect far more latency than shown." Shown in the widget, not hidden (screen 08 predates this: it shows no warning). |
| Leave | Ends the call |

States of the widget:

| State | Today |
| --- | --- |
| Connecting to the relay | "Connecting to the relay…", no round trip yet (screen 05) |
| In a call, speaking / not speaking | Lights per person (screen 03) |
| Muted | Button turns to Unmute, "Muted" label (screen 04) |
| Poor link | Red dot, high round trip (screen 08) |
| Bluetooth device | Warning in the widget |
| Call ended by the server | Banner with the reason in English, "Join again", "Close" (screen 06). The round trip shows "–" (screen 06 predates this and still shows the last one). |
| Reconnecting | **Coming soon**: the app will rejoin on its own when the network drops. Plan for a "Reconnecting…" state that keeps the people list. |
| Alone in the room | Only "you" in the list |

### 3. The latency lab (hidden)

Two tools, out of a call only (screen 09):

- **Device test**: plays clicks into the headset, times how long the
  microphone takes to hear them. Result: "Input + output: 88 ms (median of
  9 clicks)", next to what the system reports (often far less: a wireless
  headset measured 88 ms where Windows reported 28). No click heard: a
  hint to bring the earphone closer and turn the volume up.
- **Voice gate calibration**: ten seconds, controller in hand, silent.
  Result: background noise, its loudest moments, a proposed threshold;
  Apply or Discard. If the room is too loud, a warning.

Both take the sound card: they say so and refuse while in a call.

### 4. Settings (hidden)

Mute shortcut (click, then press the keys; Backspace clears it); presence
sounds volume (0 = off) with three preview buttons; the overlay on or off;
Save. The voice gate and per-person volumes are set from the call widget
and remembered.

### 5. The overlay

Pills in the top-left corner over the game (screen 11): one per person
speaking (with a light), "Muted" in red when this device is muted, and for
two seconds "{name} joined", "{name} left", "{name} lost their connection".
At most a few at once. It must stay legible over any game image, and small.

### 6. Messages to plan for

Errors the app can show, each with a short sentence (in both languages
today): leave the room first (sound card busy), files unreadable, not an
invitation link, bad server address, server refused, server unreachable,
server answer not understood (update needed), relay did not answer, sound
card could not start, not found, something went wrong, shortcut taken by
another app.

## What to hand back

- The call widget, home, and overlay, in English and French, in the states
  above (the hidden panels may be one design each).
- Colour, type, spacing and radius tokens; the current ones are CSS
  variables in `desktop/src/styles.css` and `overlay.css`, so new tokens
  can replace them one for one.
- Components (buttons, sliders, meter, person row, pill, banner) rather
  than one-off screens: the same design will be rebuilt for macOS and the
  phones.
- How the hidden parts are reached (a gear, a sheet, a drawer…), and how
  the speaking state, mute and link quality read at a glance from across
  the desk.

## Screenshot index

| File | State |
| --- | --- |
| `01-home-first-run.png` | Home before calibrating: the banner |
| `02-home.png` | Home: two spaces, invitation, device key, lab, settings |
| `03-call.png` | In a call, two people speaking |
| `04-call-muted.png` | Muted |
| `05-call-connecting.png` | Connecting to the relay |
| `06-call-ended.png` | Ended by the server |
| `07-call-default-gate.png` | Threshold still the default |
| `08-call-poor-link-bluetooth.png` | Poor link, Bluetooth devices (taken before the warning moved into the widget) |
| `09-lab-results.png` | Lab with a device test and a calibration result |
| `10-home-unreachable.png` | A space whose server is unreachable |
| `11-overlay.png` | Overlay over a game: two speakers, a lost connection |
| `12-call-fr.png` | The call in French |
