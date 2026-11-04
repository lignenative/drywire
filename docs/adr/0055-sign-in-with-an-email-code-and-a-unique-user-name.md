# 0055. Sign in with an e-mail code and a unique user name

Status: Accepted · 2026-11-04 · built in the server, the CLI, the
desktop app and the web client (canvas V4); vaults and codes not yet
removed

## Context

ADR 0051 made the account a key shared by its person's devices, moved
from one device to the next with a 26-character code. ADR 0054 added
codes that let someone create a space, and ADR 0053 safety codes to
compare keys. Each of these holds up on its own; together, on the
canvas, they make a first visit look like a key-management tool.

The maintainer, on 2026-11-04, after looking at the screens: « Je cible
des gamers qui veulent juste un onboarding et une app simples ». To get
his friends onto Drywire, signing up must work the way they already know
from Discord: an e-mail address, a unique user name, and an account in a
minute. What does not change is what they came for: no latency, and a
server that cannot listen.

Two things make that possible now:

- **Voice has no history.** A new device needs no key of the past: it
  only has to be let into each space's group from now on.
- **MLS does exactly that** (ADR 0052, steps 1 and 2 built): the app of
  any member online adds a new device to the group, which hands it the
  current epoch's secret. Nothing has to be carried by hand any more:
  not the space keys (in the vault, ADR 0051) and not the account key.

## Decision

- **An account is an e-mail address and a unique user name.**
  - The e-mail address is how a person signs in, and is never shown to
    others.
  - The user name is how others find and invite them: unique on the
    instance, 2 to 32 characters of `a`–`z`, `0`–`9`, `_` and `.`,
    compared without case, as on Discord.
  - The display name is free and not unique, as today.
- **Signing in is a 6-digit code sent by e-mail**, never a link. A link
  opens on the device that reads the mail, often the phone, when the
  person signs in on their PC; and a token in a URL ends up in logs and
  histories (no secret in a URL, the maintainer's rule).
  - The code is valid 10 minutes, for 5 tries; then a new one is needed.
    Requests are rate-limited per address and per network address.
  - The answer is the same whether an account exists or not, so the
    form tells nobody which addresses have one. A new address goes on
    to choose its user name.
  - **The system fills the code in.** The page asks for it in one field
    with `autocomplete="one-time-code"` and `inputmode="numeric"`, and
    the mail's subject starts with the code ("123456 is your Drywire
    code"): iPhone, iPad and Mac then offer the code from Mail or
    Messages above the keyboard, as the maintainer asked (2026-11-04).
    The six boxes on screen are a drawing of that one field, since six
    separate fields break the system's autofill. A pasted code with
    spaces or a dash is accepted.
  - The server keeps a hash of the code, never the code, and never logs
    it, like every token.
- **Each device has a key of its own again**, made on its first start,
  never copied. Signing in by e-mail registers the device's key with the
  account; after that it logs in with its key as today (ADR 0020), and
  the e-mail is needed again only on a new device. The settings list the
  account's devices, and any one can be signed out alone, which ADR 0051
  could not do.
- **Members, roles, bans and invitations name the account**, not a key.
  Announcements stay signed by the device's key; a member's app accepts
  a device that the server lists for a member's account and that is a
  leaf of the space's group.
- **End-to-end encryption stays, and stays out of sight.** The space's
  MLS group gives each epoch's keys to the current devices of its
  members only (ADR 0052). The relay and the server still cannot listen.
  What the person no longer does is carry keys or codes.
  - **What this costs:** the server now says which device belongs to
    which account. An operator who wanted to listen would have to add a
    device of their own to someone's account.
  - **It would show:** every member's app notes "new device" for that
    person, and the safety code (ADR 0053), kept in the advanced view,
    is computed over the pair's device keys and changes with them.
    WhatsApp and Signal make the same trade.
- **Inviting someone**:
  - **by user name**: the invitation waits in their app's home,
    "sam invites you to Team", with Join and Decline;
  - **by a short link** for someone without an account yet: it carries
    the invitation code only, never a key. The newcomer signs up, then
    joins.
- **Anyone signed in can create a space**, within the limits of the
  hosted offer (business plan). Creation codes (ADR 0054) go; a
  self-hosted instance can still restrict creation to the accounts it
  lists.
- **One instance by default.** The apps talk to the hosted instance and
  never ask which one. Choosing another is an advanced setting, for
  self-hosting.
- **E-mail sending**:
  - production sends through SMTP, with the standard library
    (`net/smtp`), to a provider in the European Union chosen in its own
    ADR, which becomes a listed sub-processor;
  - development writes each mail to a file under `.dev/mail/`: no
    secret in the repository, and no code in any log.

## What goes

- Recovery codes, device codes and vaults (ADR 0051, protocol section
  15), and the shared account key.
- Creation codes and `make creator-code` (ADR 0054).
- The instance field on every form.
- Space keys in invitation links, as ADR 0052 planned.

Instances hold test accounts only (kaseki, staging): they are reset when
this ships, rather than migrated.

## Build order

1. **ADR 0052, steps 3 and 4**: the audio path's keys from each space's
   group, and the apps driving the group. E-mail accounts need them,
   since nothing else will bring the space keys to a new device.
2. **Server**: accounts (e-mail, user name), e-mail codes and their
   limits, sending, devices of an account, members and invitations by
   account, pending invitations.
3. **Apps**: the onboarding of the canvas's V4 page, invitations by user
   name, the list of devices.
4. **Removal** of vaults and codes, and reset of the test instances.

## Consequences

- Signing up is three screens (address, code, user name) and needs
  nothing to keep safe. Signing in on a new device is two.
- **Whoever reads the person's mail can sign in as them** on a new
  device, as with most services. Their friends' apps show the new
  device. A second factor can come later.
- The e-mail address becomes personal data the instance holds:
  - its purpose is signing in;
  - it is in the export, deleted with the account, and recorded only as
    a hash in the deletion ledger, so that no restore brings it back;
  - the privacy policy names it and the sending provider.
- **Minors** (decided in the business plan, 2026-11-04): 13 at least,
  and between 13 and 15 with a parent's agreement, as French law asks
  below 15; whoever pays is an adult. No date of birth is asked: the
  sign-up says it in one sentence, and only payment has a box to tick.
- The desktop app on Windows has no system autofill for codes: the
  person types the code or pastes it. On macOS, whether the app's web
  view offers the code from Mail is to check on a real Mac.
