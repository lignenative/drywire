# 0037. Build the interface from shadcn components on Base UI

Status: Accepted · 2026-11-04

## Context

The interface was redesigned on a design canvas (CLAUDE.md, "Design
first"): a permanent side menu of spaces and rooms, a call view with
large names, popovers for the voice gate, a person and the connection,
dialogs, sheets on the phone, sliders, switches and a settings page with
its own menu. The first desktop interface was a few hand-written
components on one stylesheet (`desktop/src/styles.css`).

Rebuilding the new design by hand means writing, for every popover, menu,
dialog, slider and switch, the keyboard handling, focus trapping and ARIA
attributes that make them usable without a mouse. Those are the parts
that are easy to get subtly wrong and dull to test.

The maintainer works with React, Vite and shadcn components, and wants
every element of the interface built from one set of components so that
the app stays consistent with the mockup.

The project's rule is "standard library first, few dependencies".

## Decision

- **shadcn components on Base UI** (`shadcn init --base base`). shadcn
  copies each component's source into `desktop/src/components/ui/`, where
  we own and restyle it; Base UI underneath handles focus, keyboard and
  ARIA. Both are MIT.
- **Every element of the interface is built from these components**, or
  from components of ours that are made of them (`desktop/src/components/`).
  No hand-rolled control next to a shadcn one.
- **Components are added when a screen needs them**, not all at once.
  Several shadcn components pull a dependency of their own (date picker,
  charts, carousel, one-time code input) that Drywire has no use for.
- **Tailwind CSS**, which shadcn is written in, replaces the stylesheet.
  The canvas's colours, fonts and radii become its theme variables in
  `desktop/src/index.css`, the one place they are defined.
- **zod** checks what crosses a boundary: the Rust side's replies and,
  for the web client later, the server's. A newer or older server must
  not break the interface (version skew, CLAUDE.md), so a reply that does
  not parse becomes an error the interface can show. The two forms (an
  invitation link, a display name) use zod with plain controlled inputs,
  without a form library.
- **TanStack Query** only if the interface comes to fetch server data
  itself (the web client). No router: the app has a handful of views and
  no addresses.
- **The fonts** of the mockup (Geist and Geist Mono, OFL) are bundled with
  the app from the `@fontsource` packages, never fetched from a CDN: the
  app works offline and sends nothing to a third party.

## Consequences

- More JavaScript dependencies than before: Tailwind and its Vite plugin,
  Base UI, the small helpers shadcn components import (`cn`, shadcn's
  class merger, `class-variance-authority`, `tw-animate-css`), the Lucide
  icons, zod and the fonts. shadcn's own CSS helpers are copied into
  `desktop/src/shadcn.css` rather than installing its command-line
  package as a dependency. Each is checked by `scripts/js-licenses.ts`; Tailwind's CSS
  engine, `lightningcss`, is MPL 2.0, which is allowed.
- Two licenses join the allowed list. 0BSD (`tslib`, under Tailwind's
  WebAssembly fallback) asks even less than MIT. The SIL Open Font
  License (Geist, Geist Mono) is allowed for the bundled font packages
  only: `scripts/js-licenses.ts` refuses it anywhere else.
- The components live in the repository as source: an update of shadcn
  is a deliberate re-run of its CLI and a review of the diff, not a
  version bump.
- The interface runs in a plain browser too, against a simulated call
  (`desktop/src/api.ts` picks it when Tauri is absent), so the design can
  be checked and iterated on without building the Rust side.
