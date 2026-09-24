# ADR 0041 — One app opens the store at a time; each runs the host inside it

- **Status:** Proposed (2026-09-23), revised 2026-09-24, on
  `feature/tui-frontend`
- **Spec:** [`specs/005-tui-frontend`](../../specs/005-tui-frontend/spec.md)
  (FR-040–FR-043, User Story 6, Clarifications 2026-09-24)
- **Related:** [ADR 0038](0038-the-store-is-turso-not-sqlcipher.md) (the engine
  whose lock this obeys), [ADR 0021](0021-exactly-once-send.md) (which this
  keeps true), [ADR 0010](0010-mcp-surface.md) (which extracted
  `postio-session` so a second consumer would not be a second application)
- **Decision:** **The desktop app, the terminal app and the macOS app each
  open the store themselves, one at a time. Each runs `postio-host` in its own
  process and reaches mail only through `postio-client`. An app that finds the
  store open elsewhere says so and does not open it.**

---

## Context

The terminal frontend was asked for with "the same store available to the
GTK version and TUI". Turso, at `0.8.0-pre.11`, holds an exclusive lock on the
store file for whoever opens it. Its multi-process mode is experimental: it
refuses `VACUUM`, the store's reclaim path, and has open panics upstream. The
operation queue's claim is correct only because one drainer exists, and
`LocalStore`'s caches assume one writer.

The first version of this decision, on 2026-09-23, read the request as both
apps open at once. It put the store in a background process,
`postio-daemon`, with every frontend a client over a Unix socket. That was
built and worked, and the maintainer withdrew it on 2026-09-24 as too much
complexity for what it bought: "Let's set it up so we can only run one app at
a time but it can be either or."

## Decision

1. **One app at a time.** Whichever app starts first opens the store, runs
   sync, drains the operation queue and does the store's upkeep. It stays the
   only one until it quits. There is no background process.
2. **The other app waits its turn.** An app that finds the store open
   elsewhere tells the person to close the other one. It does not modify the
   store, start sync, or queue anything.
3. **One implementation, in-process.** Every app runs `postio-host` inside its
   own process and reads and writes through `postio-client`'s in-process
   transport. That is the one place each store operation is written: the list
   and its paging, reading, search, compose, settings, onboarding and
   notifications. The desktop, terminal and macOS frontends all use the same
   code. The in-process call is a spawn and a oneshot with no encoding.
4. **One store, wherever the app came from.** A source install and both
   Flatpaks use the same `~/.local/share/postio`, so the mailbox is the same
   whichever app opens it.

## Consequences

- There is nothing to start, find, keep alive, reconnect to or version-match
  across processes: no socket, no handshake, and no grace period.
- The two apps cannot be open at the same time. Someone who wants the
  terminal while the desktop app is open has to close the desktop app first.
- The host/client split stays. It exists for sharing the implementation, not
  for crossing a process. If running both at once is ever wanted again, a
  transport over a socket is the part to add back; the first version of this
  ADR is its design, in `git log` on this file.

## Revisit when

Turso's multi-process mode supports `VACUUM`, closes its open panics, and
offers a cross-process commit signal; or someone asks again for both apps at
once and the daemon's cost is worth paying.
