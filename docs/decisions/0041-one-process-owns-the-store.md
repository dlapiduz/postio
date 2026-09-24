# ADR 0041 — One process owns the store; frontends are its clients

- **Status:** Proposed (2026-09-23), on `feature/tui-frontend`
- **Date:** 2026-09-23
- **Spec:** [`specs/005-tui-frontend`](../../specs/005-tui-frontend/spec.md)
  (FR-040–FR-043, User Story 6); the reasoning in full is its
  [research.md](../../specs/005-tui-frontend/research.md) R1–R2
- **Related:** [ADR 0038](0038-the-store-is-turso-not-sqlcipher.md) (the engine
  whose lock this obeys), [ADR 0021](0021-exactly-once-send.md) (which this
  keeps structurally true), [ADR 0013](0013-event-fanout.md) (the fan-out a
  socket connection subscribes to), [ADR 0010](0010-mcp-surface.md) (which
  extracted `postio-session` so a second consumer would not be a second
  application)
- **Decision:** **Exactly one process opens the store: `postio-daemon`, built
  from `postio-host`. Every Linux frontend reaches mail only through
  `postio-client`, over a user-private Unix socket. A frontend may run the host
  in-process only where no other frontend can share the store (the macOS
  frontend, and tests).**

---

## Context

The maintainer asked for the GTK app and a new terminal frontend to run at the
same time on one store (2026-09-23). Turso, at `0.8.0-pre.11`, holds an
exclusive file lock unless opened in an experimental multi-process mode. That
mode refuses `VACUUM`, which is the store's reclaim path, and has open panics
upstream. Separately, the operation queue's claim is an unconditional write
that is correct only because one drainer exists, and `LocalStore`'s caches
assume one writer.

## Decision

1. **One owner.** `postio-daemon` owns the store, the blob directory, the key,
   the engines, the queue drainer, the event hub, undo and egress recording.
   No other process opens the store file, including diagnostics, which ask
   the daemon.
2. **Frontends are clients.** `postio-app` and `postio-tui` depend on
   `postio-client` and never on `postio-storage`, `postio-runtime` or the
   engine. `check-crate-boundaries.py` enforces this for every frontend crate.
3. **Nothing on the protocol waits on the network.** Requests are answered from
   local state; remote effects are commands, and their outcomes are events.
4. **Client and daemon come from one build.** The handshake refuses any
   mismatch rather than negotiating.
5. **User-private transport.** `$XDG_RUNTIME_DIR/postio/`, `0700`/`0600`,
   peer uid checked, never TCP.

## Consequences

- The store-side logic in `postio-app` moves into `postio-host` once, and both
  frontends consume it. That is the "logic once" the frontends needed anyway.
- A frontend's cold start may include spawning the daemon. The 500 ms budget
  still applies and is measured.
- Adding a frontend means adding a client, not a second application sharing a
  file.

## Revisit when

Turso's multi-process mode supports `VACUUM`, closes its open panics, and
offers a cross-process commit signal. Even then, one drainer remains the
simpler guarantee for exactly-once.
