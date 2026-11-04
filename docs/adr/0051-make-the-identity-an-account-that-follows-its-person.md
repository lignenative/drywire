# 0051. Make the identity an account that follows its person

Status: Accepted · 2026-11-04

## Context

An identity is an Ed25519 key pair made by each device on its first
start, and it is the only thing the server knows a person by (spec
section 8, ADR 0020). That kept Drywire free of passwords and of any
secret on the server, but the first tries on real devices showed what it
costs:

- **Every device, and every browser, is someone else.** Chrome, Safari
  and Firefox on one Mac are three members of a space, each with its own
  name, each using up an invitation.
- **Losing a device loses everything**: the identity, the key of every
  space it joined (which only the device holds), and the spaces it owned,
  which nobody else can then manage.
- **Nothing lets a person sign in on a new device.** The only way into a
  space is a new invitation, from someone else.

The maintainer asked for "real accounts, like Discord's". What Discord
gives that Drywire lacks is the account that follows its person from one
device to the next. What Drywire must keep is what Discord does not
promise: the server never holds a key that opens the audio, and needs no
password or e-mail address.

Two ways to make one person of several devices:

- **One key per device, certified by an account key** that the person
  keeps somewhere safe. Each device can be revoked alone, but every
  announcement, every member list and every client must learn to follow
  certificates from device to account, and the wire format changes.
- **One key per person, shared by their devices.** The identity key
  becomes the account's; a new device receives it, encrypted, from a
  device that has it or from a sealed copy the person can open. Nothing
  changes on the wire: announcements, logins, members, bans and presence
  already name that one public key. A device cannot be revoked without
  changing the account's key, which Signal's linked devices accept too.

## Decision

- **An account is the identity key, shared by its person's devices.** The
  server still knows one public key per person, as now. The wire format,
  the login (ADR 0020), the announcements and the member lists do not
  change, and the server stays able to read none of them.
- **A vault carries the account to another device.** It is the account's
  private key, display name and spaces (instance, id, name and space key
  of each), sealed with AES-256-GCM under a key derived from a **code**:
  128 random bits, shown as 26 characters in groups (Crockford base32).
  The code also derives the vault's id, under which an instance keeps
  the sealed bytes. The instance stores them, never the code, and cannot
  open them (protocol section 15). A recovery vault is kept on every
  instance where the account has a space, so any of them can give it
  back; the person names the instance with the code (the web client
  served by an instance already knows it).
- **Two kinds of code**, one mechanism:
  - a **recovery code**, made when the person asks for it, shown once,
    kept by them (on paper, in a password manager). The instance keeps
    its vault until the account is deleted, and the device that made it
    seals it again whenever its spaces change. Making a new code replaces
    the old vault: the old code opens nothing any more.
  - a **device code**, to add a device now: valid ten minutes, opened
    once, then gone, on the one instance shown beside it.
- **Opening a vault needs no login**, since the new device has no key
  yet: it sends the id, never the code, and gets the sealed bytes. The id
  is 256 bits nobody can guess; the request is rate-limited like a login.
- **A device that opens a vault becomes that account**: it keeps the key
  and the spaces, and joins them as the same person. A device that
  already has an account with spaces asks before replacing it.
- **One person, one voice.** An identity already holds one session on an
  instance (ADR 0021): joining a room from a second device ends the
  first's, which is what a person switching from their PC to their phone
  wants. An identity now holds 8 tokens instead of 4, so that a few
  devices stay logged in together.
- **In the browser**, the account key is no longer made non-extractable
  (ADR 0040): the page keeps the 32-byte seed in IndexedDB, to seal it in
  a vault, and imports it as a non-extractable signing key for use. A key
  made by an older version, which cannot be read, keeps working but
  cannot be put in a vault; the page says so.
- **Not now**: unique user names (nothing needs to find a person by name
  yet: invitations bring people in), passwords (a 128-bit code needs no
  key stretching and cannot be guessed; a password would), sign-in with a
  passkey (support for deriving a key from one varies by browser and
  system; to check before building), and an e-mail address (only the
  hosted offer's billing needs one, in its own repository, ADR 0043).

## Consequences

- A person has one name and one membership per space, whatever devices
  they use, and gets their spaces back on a new device with a code, or
  from a device beside them.
- **Whoever holds the code holds the account**: the recovery code is as
  sensitive as all the person's devices together, and the apps say so
  when they show it. A device code is worth the same for ten minutes.
- A lost or stolen device cannot be shut out alone; the person makes a
  new account (and is invited again) if they fear the old key leaked.
  Revoking one device needs per-device keys, left for when it is worth a
  wire format change.
- The instance stores one more thing per account: a sealed vault of a
  few kilobytes, which it cannot read. It is personal data all the same:
  in the export (as its id and size), deleted with the account, and
  listed in spec section 9.
- A device that replaces its account forgets the old key, but not the
  instances: the old account stays a member of its spaces, silent, until
  someone removes it or the inactivity purge does (spec section 9).
  Deleting it there would hand over or delete the spaces it owned, which
  the person did not ask for.
- The space keys still travel in invitation links; how that changes is
  ADR 0052.
- The web client's identity is as safe as the page's integrity, as it
  was in practice: a script that runs in the page could already sign
  with the key while the page was open.
