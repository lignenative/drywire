# 0054. Let the operator hand out codes that create a space

Status: Accepted · 2026-11-04

## Context

Who may create spaces on an instance is the operator's choice: an
instance is not an open sign-up (spec section 9). Until now the operator
said it with public keys in `DRYWIRE_SPACE_CREATORS`: ask the person for
the 64 hex digits their device shows, paste them into `instance.env`,
deploy again. It worked for the maintainer's own key and nobody else's:
the person must find a key buried in the settings, and every new creator
costs a deploy. Since ADR 0051 a key is a person's account, so the list
still makes sense, but it is no way to let a friend start their space.

## Decision

- **A creation code** lets one person create one space. The operator
  makes it on the host with `drywire-server creator-code` (`make
  creator-code` from their machine) and hands it over like an invitation,
  privately. It is 128 random bits, written as the 26 characters of an
  account code (protocol section 15.1), so a person types every Drywire
  code the same way.
- **The server keeps its SHA-256 only**, as for invitation codes, with an
  expiry 7 days later, and nothing about who uses it. Creating a space
  with it (`creator_code` in `POST /v1/spaces`) spends it in the same
  transaction: one code, one space. An unknown, used or expired code all
  answer the same `404`; the daily purge drops the expired ones.
- **The list stays**: keys in `DRYWIRE_SPACE_CREATORS` create spaces
  without a code, as before. The operator lists their own, and hands
  codes to everyone else.
- The command prints, besides the code, the web client's address that
  opens the form with it (`https://<instance>/#create=<code>`), since
  every instance serves its web client (ADR 0049).
- The apps gain the two screens that make this useful: **Create a space**
  (beside Join a space, with the code) and **Invite people** (for owners
  and admins), drawn on the canvas first.

## Consequences

- An operator never has to ask anyone for a key or redeploy to let them
  create a space.
- A code is worth one space for a week: one that leaks lets a stranger
  make a space on the instance, which the operator can delete. It never
  gives access to anyone's existing space.
- The command writes to the database while the server runs, through
  SQLite's own locking: no restart, nothing to reload.
