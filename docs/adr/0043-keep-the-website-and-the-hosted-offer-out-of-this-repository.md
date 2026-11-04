# 0043. Keep the website and the hosted offer out of this repository

Status: Accepted · 2026-11-04

## Context

Drywire is meant to launch publicly with a website: a landing page, a
pricing page for a hosted offer (self-hosting stays free), and later the
billing and accounts that a paid offer needs. ADR 0042 (commit cf1f642)
moved `site/` to Astro so that it could hold those pages, and was
reverted before anything was built on it.

Those pages are not the software. They carry Ligne Native's brand, its
prices and its sales copy, which change for business reasons and which a
fork has no use for. Billing would bring payment providers, customer
records and secrets that an open-source repository should not have to
carry or guard. Mixing them with the code would also blur what the MIT
license covers: the code, not the name or the logo.

The invitation page is different. Every invitation link, whichever
instance it is for, points to it (spec section 8), so every self-hosted
instance depends on it, and its promise (it sends the link's secret
nowhere) is something anyone must be able to read and check.

## Decision

- **A separate, private repository** holds the project's website: the
  landing page, the pricing page, the brand assets, and later the billing
  and the accounts of the hosted offer.
- **This repository keeps the software** (server, core, clients) **and
  the invitation page**, `site/i/`, in plain HTML, CSS and JavaScript with
  no build step, as before ADR 0042. One page needs no framework, and
  keeping it dependency-free keeps it auditable at a glance.
- **The code is MIT; the Drywire name and logo are not.**
  [`TRADEMARKS.md`](../../TRADEMARKS.md) says what that means for forks.
- The website serves the invitation page from this repository's
  `site/i/`, at `/i` on the project's domain; the private repository
  never forks or rewrites it.

## Consequences

- Nothing here depends on the hosted offer. Per-space quotas stay
  possible in the data model (spec section 15), but payment, plans and
  customer accounts are never part of the server.
- A contributor sees the whole product in this repository except its
  marketing pages; changes to the invitation page are reviewed here, in
  public.
- The design canvas still covers both: the website's mockups are drawn
  there, and its code follows them in the private repository.
- Publishing the site means assembling two sources: the private website
  and `site/i/` from here. The private repository documents how.
