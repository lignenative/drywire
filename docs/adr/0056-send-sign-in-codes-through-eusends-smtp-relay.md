# 0056. Send sign-in codes through eusend's SMTP relay

Status: Accepted · 2026-11-10 · nothing to build; the hosted instance
waits for its account, DNS records and key

## Context

ADR 0055 signs devices in with a 6-digit code sent by e-mail, and left
the provider to its own decision. The server already sends through any
SMTP submission port over TLS, with the standard library
(`server/internal/mail`), or writes the mails to a directory in
development.

The maintainer chose eusend.dev, on 2026-11-10: « eusend.dev ». What its
pages say (read on 2026-11-10, eusend.dev/docs and eusend.dev/legal):

- **Sending**: an HTTP API on the model of Resend's (`POST
  https://api.eusend.dev/emails`, a bearer key), and an **SMTP relay**:
  `smtp.eusend.dev`, port 465, TLS from the first byte only (no
  STARTTLS), user name `eusend`, the API key as password. The sending
  address must be on a verified domain.
- **Checked from this Mac, 2026-11-10**: the relay answers on 465 over
  TLS 1.2 with a valid certificate, offers `AUTH PLAIN LOGIN` and
  `8BITMIME`, and refuses a client that speaks before its greeting.
- **Keys**: `eu_live_…` sends; `eu_test_…` runs every check and
  delivers nothing.
- **Free plan**: 3,000 mails a month, 100 recipients a day, one domain;
  paid plans from 10,000 mails for €9 a month.
- **Domain**: a DKIM key in TXT at `eusend._domainkey.<domain>`; no SPF
  on the root; optionally, for SPF alignment, `send.<domain>` with TXT
  `v=spf1 include:_spf.eusend.dev ~all` and MX `feedback.eusend.dev`
  (priority 10); DMARC recommended.
- **Who runs it**: "Bjørnerås Labs, a registered Norwegian sole
  proprietorship", organisation number 938 162 336, in the EEA. A data
  processing agreement under GDPR article 28 is published
  (eusend.dev/legal/dpa); customer data is "stored inside the European
  Union" (eusend.dev/legal/security). The sub-processors page names
  categories: EU hosting for the database, storage and compute,
  Cloudflare for network transit, Polar for billing, error monitoring.
- **What it keeps**: a log of 30 days on the plans below 250,000 mails:
  status, timestamps, addresses, subject and the rendered body.

## Decision

- **The hosted instance sends its sign-in codes through eusend's SMTP
  relay**, with the sender the server has. No code and no dependency:
  `instance.env` names the relay, the user name, the live key and the
  sending address:

  ```text
  DRYWIRE_SMTP_ADDR=smtp.eusend.dev:465
  DRYWIRE_SMTP_USER=eusend
  DRYWIRE_SMTP_PASSWORD=eu_live_…
  DRYWIRE_MAIL_FROM=Drywire <no-reply@…>
  ```

  A test against an in-process relay that behaves like eusend's (TLS
  from the first byte, AUTH PLAIN, a greeting to wait for) checks the
  sender (`server/internal/mail/smtp_test.go`).
- **Not the HTTP API**: it would need a client of our own for what SMTP
  already does. Its idempotency key would not help here: a person who
  asks for a code again gets a new one.
- **The key is a secret**: in `instance.env` on the host only, never in
  the repository or a log. The server logs no provider error, since one
  often quotes the address.
- **Staging** uses a live key too, since the maintainer signs in there
  with his own address; a test key delivers nothing.

## Consequences

- The maintainer creates the eusend account, adds the sending domain and
  its DNS records, and puts the live key in the instance's
  `instance.env`. Nothing changes in the code.
- eusend becomes a sub-processor of the hosted instance: it sees each
  address and code it sends, and keeps them 30 days in its log. A code
  works 10 minutes and once, so the log holds no usable one.
- **Before production**, for the register of processing and the privacy
  policy, three facts are still missing from eusend's pages: its postal
  address, the names and countries of its hosting providers, and how its
  data processing agreement is accepted (none of the pages read shows a
  signing step). To ask privacy@eusend.dev.
- The free plan's 100 recipients a day is the limit that matters: one
  code per sign-in on a new device. Past it, the paid plans start at €9.
- Any other provider with an SMTP port over TLS fits the same way, by
  `instance.env` alone.
