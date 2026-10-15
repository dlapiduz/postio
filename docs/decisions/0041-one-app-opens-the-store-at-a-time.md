# ADR 0041 — One app opens the store at a time; each runs the host inside it

- **Status:** Accepted (2026-09-25). Built. **Amended 2026-10-14**: the app with the store
  answers agents on a private socket; see **Amendment** below
- **Spec:** [`docs/archive/specs/005-tui-frontend`](../archive/specs/005-tui-frontend/spec.md)
  (FR-040–FR-043, User Story 6, Clarifications 2026-09-24)
- **Related:** [ADR 0038](0038-the-store-is-turso-not-sqlcipher.md) (the engine
  whose lock this obeys), [ADR 0021](0021-exactly-once-send.md) (which this
  keeps true), [ADR 0010](0010-mcp-surface.md) (which extracted
  `postio-session` so a second consumer would not be a second application)
- **Decision:** **The desktop app (Focus, ADR 0043), the terminal app and
  the macOS app each open the store themselves, one at a time. Each runs `postio-host` in its own
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

The maintainer chose one app at a time over both at once: "Let's set it up
so we can only run one app at a time but it can be either or."

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
- No two apps can be open at the same time. Someone who wants the terminal
  while the desktop app is open has to close the desktop app first.
- The host/client split stays. It exists for sharing the implementation, not
  for crossing a process. If running both at once is ever wanted, a transport
  over a socket is the part to add.

## Rejected

- **A background store process** (`postio-daemon`), every frontend a client
  over a Unix socket. It was built and worked, and it was too much complexity
  for what it bought: something to start, find, keep alive, reconnect to and
  version-match. Its design is in `git log` on this file.

## Revisit when

Turso's multi-process mode supports `VACUUM`, closes its open panics, and
offers a cross-process commit signal; or someone asks again for both apps at
once and the daemon's cost is worth paying.

---

## Amendment (2026-10-14): the app with the store answers agents

**Why.** `postio-mcp` (ADR 0010) runs in its own process, and under this
ADR it could only say "Postio is already open in another window" while an
app was open. The maintainer keeps the Mac app open all day, so the agent
was useless exactly when the person was at their mail. The maintainer asked
for this limit to be removed.

**Decision.** The app that holds the store listens on a Unix domain socket
beside it, `agents/mcp.sock`, and answers agents' tool calls itself.
`postio-mcp` tries that socket first for every `tools/call`. It opens the
store itself, as before, only when no app is listening.

- **What crosses is MCP's JSON-RPC, one line each way, not the client
  protocol.** Frontends still reach the host in-process, with no encoding,
  as decided above. The app answers each line with the same
  `postio_mcp::Server` that `postio-mcp` runs, over its own host
  (`postio_mcp::socket`). This adds no API: JSON-RPC is already the wire
  format, and there is still exactly one implementation of every tool.
- **The checks run where the store is.** The app reads `[mcp]` from its own
  `config.toml` on every call and writes the access log beside the store.
  `postio-mcp`'s own `--config` does not apply to calls the app answers. The
  app is the process that owns the mail, so it is the one that enforces
  access to it. The checks stay in one place, whichever process answers.
- **The agent acts beside the window.** `Connector::connect_agent` connects
  the agent's client to the verbs (selection and undo stack) of the app
  window that connected last, and forwards to that window the feedback the
  agent's commands produce. The window therefore repaints, shows "Archived 1
  message — Undo", and `u` / ⌘Z takes the change back, exactly as for the
  person's own keystroke. Tracked sends still report back to the agent.
  This was tried on the Mac demo: an archive made over MCP left the open
  list at once with the undo toast showing, and ⌘Z brought it back.
- **Who can connect.** There is no TCP and no port. The socket's folder is
  `0700` and the socket is `0600`. Both ends read the other end's user id
  from the kernel (peer credentials: `getpeereid` on macOS, `SO_PEERCRED` on
  Linux, through tokio) and refuse any other user. `postio-mcp` also refuses
  a socket, or a folder, that another user owns or that others can reach.
- **Every app answers.** The desktop app (`startup::open`), the terminal
  (`run`) and the Mac app (`postio-ffi`'s `Session::open` over a store file)
  each listen while they hold the store. They stop listening and remove the
  socket on shutdown. A socket left by a crash answers nothing, so the next
  app replaces it. A socket that something still answers on is never taken
  over. An in-memory session and the desktop app's demo do not listen. The
  Mac demo listens only when `POSTIO_DEMO_AGENTS=<socket>` says so.

**Rejected.** A serde encoding of `postio-client`'s `Req`/`Resp`, so that
`postio-mcp` could be a remote client of the app's host. It would have
meant ninety request kinds and every model type they carry crossing a
process boundary, and versioning all of it, only so that `postio-mcp` could
run the tools in its own process. Running them in the app costs nothing,
because the app already links the same code.

**Still true.** One app holds the store. The socket is how other processes
reach that app; it does not let them hold the store too. Running the
desktop app and the terminal at the same time still needs the daemon this
ADR rejected.
