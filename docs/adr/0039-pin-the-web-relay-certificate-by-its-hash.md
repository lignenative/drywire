# 0039. Pin the web relay's certificate by its hash

Status: Accepted · 2026-11-04

## Context

WebTransport runs over TLS 1.3. In production, the control plane sits
behind a reverse proxy that terminates TLS (ADR 0027), so the server
itself holds no certificate, and a proxy cannot pass WebTransport
through. Giving the relay a certificate from a certificate authority
would mean sharing the proxy's files, or running ACME in the server, and
reloading on renewal. In development there is no authority at all.

Browsers have a way around this: a page may name the SHA-256 hashes of
the certificates it accepts (`serverCertificateHashes`), and the browser
then checks the hash instead of a chain. Chrome 100, Firefox 125 (136
for a fix), Safari 26.4. The certificate must use an ECDSA P-256 key, be
valid for two weeks at most, and be valid now
([W3C WebTransport](https://www.w3.org/TR/webtransport/#custom-certificate-requirements),
[MDN](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport/WebTransport)).

The control plane is already the root of trust for the relay: clients
reach it over real TLS, and it hands them their session secret.

## Decision

- The relay's WebTransport side **makes its own certificates**: ECDSA
  P-256, valid 13 days from an hour before they are made.
- It presents one for 6 days and keeps the **next one ready**. The join
  answer lists both hashes (`web_relay.certificate_hashes`, protocol
  section 13.6), so a browser that joined just before a rotation still
  connects.
- The page passes the hashes to `serverCertificateHashes`, with the
  algorithm spelled `sha-256` in lowercase, as some engines compare it
  literally.
- The same mechanism serves development and production: no certificate
  to manage, no ACME, nothing to reload.

## Consequences

- The hashes are only as trustworthy as the control plane's TLS. A
  browser that reached the control plane over plain HTTP, which only
  happens on loopback in development, trusts whatever it was told.
- Certificates live in memory: a restart makes new ones, and browsers
  get the new hashes when they join again, as they do after any restart.
- `allowPooling` must stay off with hashes, so each tab opens its own
  QUIC connection. A Drywire page needs one.
- A future Content Security Policy rule may require
  `'unsafe-webtransport-hashes'` in the page's `connect-src`; the web
  client's own policy will include it.
