# 0031. Protect the desktop app's identity with DPAPI

Status: Accepted · 2026-10-28

## Context

A device's Ed25519 seed is its identity: whoever reads it can speak as
that device on every instance. The space keys from invitation links are
as sensitive: whoever reads one hears that space. The spec (section 8)
asks the apps to keep them in the system's secure storage. The
command-line client keeps them in files only their owner can read.

The desktop app ships on Windows first (phase 3). It also runs on macOS,
where it is developed, but the macOS app people will install is the
SwiftUI one of phase 4. Linux comes in phase 6.

Options on Windows:

- **DPAPI** (`CryptProtectData`, user scope): encrypts with a key derived
  from the user's logon credentials. Only that user, on that machine, can
  decrypt; a copied file is useless elsewhere. A plain Win32 call, in the
  `windows` crate the core already uses.
- **The Credential Manager**: built on DPAPI, but meant for passwords, with
  a size limit per entry and entries a user can browse and delete without
  knowing what they are.
- **A keyring crate** wrapping both platforms: one more dependency for two
  calls.

## Decision

- On Windows, the app seals `identity.json` and `spaces.json` with DPAPI,
  in user scope, with a fixed application string as extra entropy, so
  another program of the same user has to know it to call DPAPI on our
  files. The files sit in the app's data directory
  (`%APPDATA%\com.lignenative.drywire`).
- Each file names the protection it was sealed with (`"dpapi"` or
  `"none"`), so a file copied to another system is refused with a clear
  message rather than read as garbage.
- On macOS and Linux, for development only, the files are not encrypted:
  they are written readable by their owner only (0600, in a 0700
  directory), as the command-line client does.
- The desktop app has its own identity, apart from the command-line
  client's: the CLI's files are not protected, and mixing the two would
  lower the app's protection to the CLI's.

## Consequences

- An attacker running code as the same Windows user can still call DPAPI
  and decrypt, as with any user-scope secret store; DPAPI protects against
  other users, offline copies and backups, not against malware in the
  session.
- Reinstalling Windows, or moving to another PC, loses the identity: the
  person joins their spaces again with a new link, as on a new device.
- Before the desktop app ships on Linux (phase 6), the Secret Service
  (libsecret) replaces the plain file there. The macOS build of this app
  is never distributed; the phase 4 app uses the Keychain.
- DPAPI is checked by the Windows CI job, which runs the vault and store
  tests on Windows; a human checks that a copied file does not open on
  another account (`desktop/README.md`).
