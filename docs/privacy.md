# Record of processing activities (template)

The GDPR (article 30) asks every data controller to keep a record of what
it processes. The operator of a Drywire instance is the controller: the
person who self-hosts it, or Ligne Native for the hosted instance. This
template describes what `drywire-server` does; an operator copies it,
fills in the bracketed parts, and keeps it up to date.

Everything below is checked against the code: when the code changes what
it stores or for how long, this file changes in the same commit.

## Controller

- Name and contact: [operator's name, postal address, e-mail]
- Data protection contact, if any: [name, e-mail]

## Processing 1: providing the voice service

| | |
| --- | --- |
| **Purpose** | Let members of a space find each other and talk |
| **Legal basis** | Providing the service the user asked for (art. 6(1)(b)) |
| **People** | Members of the instance's spaces |
| **Data** | Ed25519 public key, display name, memberships and roles, invitations created, bans (a public key per space), last login time |
| **Not collected** | No e-mail, no phone number, no password, no audio (end-to-end encrypted, never stored, the server cannot decrypt it) |
| **Where** | `drywire-server`'s SQLite database (`DRYWIRE_DB`) |
| **Retention** | Until the identity is deleted by its owner; identities without a login for 12 months are purged daily (`store.InactiveAfter`). Invitation codes: deleted once expired or used up. A ban lasts until an admin lifts it. |
| **Recipients** | Other members of the same space see the display name, public key and role |
| **Processors** | [hosting provider, country] |
| **Transfers outside the EU** | [none, or which and under what safeguard] |

## Processing 2: routing audio and showing presence

| | |
| --- | --- |
| **Purpose** | Send each audio packet to the other members of a room; show who is in which room |
| **Legal basis** | Art. 6(1)(b) |
| **Data** | IP address and port, session number, who is in which room |
| **Where** | Server memory only |
| **Retention** | The duration of the session; nothing is written to disk. A session ends 30 seconds after the last packet. |

## Processing 3: security logs

| | |
| --- | --- |
| **Purpose** | Detect and date incidents (art. 33 notifications) |
| **Legal basis** | Legitimate interest in the security of the service (art. 6(1)(f)) |
| **Data** | Access log: time, full client IP address, method, route pattern (never the path, query string, headers or `Referer`), status, duration |
| **Where** | `DRYWIRE_ACCESS_LOG` (standard output by default) |
| **Retention** | 1 year at most: [how the operator rotates and deletes it] |
| **Not logged** | Application logs hold no IP address, token, key, name or content |

## Processing 4: backups and the deletion ledger

| | |
| --- | --- |
| **Purpose** | Restore the service after a failure, without bringing back data deleted since the backup |
| **Data** | A copy of the database (processing 1); the deletion ledger holds the SHA-256 of deleted public keys and the ids of deleted spaces, with dates |
| **Where** | Backups: [encrypted, where, off the server]. Ledger: `DRYWIRE_LEDGER`, beside the database, never in a backup |
| **Retention** | Backups: 6 months at most. Ledger entries: 7 months (pruned at every start) |

## Rights of the people concerned

| Right | How |
| --- | --- |
| Access and portability | In the app: export as JSON (`GET /v1/me/export`) |
| Rectification | In the app: rename (`PATCH /v1/me`) |
| Erasure | In the app: delete the identity (`DELETE /v1/me`), or leave a space |
| Objection, restriction | By contacting the operator, answered within one month |

## Security measures

- Control plane over TLS 1.3 only; plain HTTP is refused beyond this
  machine. Audio is end-to-end encrypted; the relay holds no key.
- Login by signed challenge; tokens last one hour and live in memory only.
- Invitation codes are stored as their SHA-256; the space key never
  reaches the server (it travels in the link's fragment).
- Requests are rate-limited per address; spaces have a member limit;
  bans by public key.
- [Server access: who has SSH access, how backups are encrypted.]

## Minors

In France, users under 15 need parental consent. [For a private instance
among adults, state it; for an instance open to teenagers, describe the
consent process and the impact assessment before opening.]
