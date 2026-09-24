# Contract: client ↔ daemon

`postio-client` is what every frontend holds. `postio-host` serves it, either
in-process or as `postio-daemon` over a socket. This is the contract between
them. It is private to one build: client and host always ship together, and a
mismatch is refused, never negotiated.

## Transport

- **Socket**: `$XDG_RUNTIME_DIR/postio/daemon.sock`, directory `0700`,
  socket `0600`. `$POSTIO_RUNTIME_DIR`, when set, names the directory
  instead: a scratch run (`scripts/run-isolated.sh`) must not reach the
  daemon that owns the real store. Never TCP. A connecting peer's uid is checked with
  `SO_PEERCRED` and must equal the daemon's.
- **Lock and pid**: `$XDG_RUNTIME_DIR/postio/daemon.lock`, held with `flock`
  for the daemon's lifetime. Turso's own file lock is the backstop.
- **Framing**: a length-prefixed frame (u32 big-endian) carrying `Frame` as
  **JSON** (`serde_json`), decided in T008. Several model types deserialize
  by hand (`flag.rs`, `ids.rs`, `operation.rs`, `action.rs`), and a format
  that is not self-describing, such as postcard, cannot always read those
  back. JSON is already in the graph, and it costs microseconds for a page of
  rows. Frames longer than `MAX_FRAME` (256 MiB) are refused before they are
  read.
- **In-process**: the same `Frame` values over channels, with no encoding.
  Used by `postio-ffi` and the integration suites.

## Starting and finding the daemon

1. The client connects to the socket.
2. If nothing answers, it takes `daemon.lock` briefly to see whether a daemon
   is starting. If none is, it spawns `postio-daemon` (resolved next to its
   own executable, then on `PATH`), detached, and retries with backoff, for
   up to 2 s.
3. If that fails, the frontend says so in a sentence ("Postio's background
   service did not start: …") and exits. It never opens the store itself.

## Handshake

```
C→H  Hello { build: BuildId, kind: Gtk|Tui|Ffi|Test, protocol: u32 }
H→C  Welcome { client: ClientId, host_build: BuildId }
   | Refused { reason: VersionMismatch { host, client } | NotOwner | Starting }
```

`BuildId` is the crate version plus the git commit. Any difference is
`VersionMismatch`, and the frontend names both versions.

## Frames after the handshake

```
Request  { id: u64, body: Req }     C→H
Response { id: u64, body: Resp }    H→C   exactly one per Request
Event    { event: postio_core::Event }  H→C   unsolicited, in order
Notify   { notification: Notification } H→C   only to the elected notifier
```

Every `Req` is answered from local state. **No `Req` waits on the network**
(Principle I). Anything remote is a `Command`, which is enqueued, and its
outcome arrives later as `Event`s.

### `Req` families

The API surface as built. `crates/postio-client/src/protocol.rs` is the
authority; `Req::family()` names each variant's family for the round-trip
counts. Each is answered by logic that already existed, moved into the host
rather than rewritten.

| Family | Requests | Answered from |
|---|---|---|
| Commands | `Send(Command, aim)`, `SendTracked(Command, aim)` | the per-client dispatcher over `postio_session::actions` |
| Lists | `Page`, `Count`, `Rows`, `RowsIn`, `NoteRemoved`, `Mailboxes`, `DraftCounts`, `Accounts` | `MailStore` |
| Reading | `Body`, `Conversation`, `Readings`, `ThreadReadings`, `StoredBody`, `InlinePart`, `Unsubscribe`, `Parts`, `SavePart`, `SaveParts`, `OpenPart`, `ExportMessages` | `postio-host/src/{reading,parts,export}.rs` |
| Search | `Search`, `SearchHits`, `Facets`, `Correspondents`, `Labels` | `postio-host/src/search.rs` over `postio_session::search` |
| Compose | `SaveDraft`, `QueueSend`, `DiscardDraft`, `RecoverDraft`, `DraftBehind`, `CancelSend`, `SendFailure`, `Recipients`, `ReplySource`, `DefaultSignature`, `Attach { path, mime_type }`, `InlineImage`, `AttachmentBytes` | `postio-host/src/compose.rs`, one `DraftWriter` per client |
| Accounts | `Discover`, `AddAccount`, `SaveAccount`, `SaveOAuthAccount`, `BeginOAuth`, `FinishOAuth`, `CancelOAuth`, `Account(AccountOp)`, `EditAccount`, `RebuildIndex` | `postio-host/src/onboarding.rs`, `postio_session::onboarding` |
| Settings | `AccountSettings`, `SaveSignature`, `DeleteSignature`, `SetBackfillExcluded`, `EgressLog`, `PrivacyLog`, `OrientationSeen`, `RetireOrientation` | `postio-host/src/settings.rs` |
| Diagnostics | `Diagnose(report)` | `postio_session::diag` |
| Startup and upkeep | `StartupRoute`, `Wired`, `StartSync`, `FetchBody(message)`, `StorageCeiling(max)`, `RecordEgress(event)` | `postio-host/src/{startup,maintenance}.rs` and the host's engines by account |
| Notifications | `Attention(Attention)`, posted and never awaited | the client's entry in the host, read by `postio-host/src/notify.rs` |

The terminal's settings are its `config.toml` (edited in `$EDITOR` at the
section) plus the account commands, so the `Settings`/`PatchSettings` pair
first planned here was not needed. A draft carries `body_markdown` when it
was typed as Markdown; the host stores what it is given and derives nothing.

### Undo

`Command::Undo` from a client undoes the top of **that client's** stack
(research R1a). The `UndoAvailable`/`Undone` events are sent only to the
client that owns the entry. Every other event goes to every client.

### Notifications

The host decides with `postio_ui::notify::decide`, unchanged, then sends
`Notify` to exactly one client: the first connected `Gtk` client, otherwise
the first `Tui` client. With no clients there is no notification.

Whether the person is already looking at the folder is the one input only a
frontend can see, so each client posts `Attention { showing, active }` when it
changes; the host decides with the elected client's latest, and with none
posted the arrival is told. The folder gate is `[sync] notify` and
`notify_roles`, read by the daemon. The wording is `Wording::Newest` (sender
and subject) for every frontend. A frontend that is not reading its
notifications loses them rather than stalling its events: at most 16 wait.

## Lifetime

- The daemon starts serving once the store is open. Before that, a `Hello`
  gets `Refused(Starting)` and the client retries.
- When its last client disconnects, it waits 30 s (research R1b). A new
  client cancels the wait. Otherwise it stops the engines (as `stop_retained`
  does today, `crates/postio-app/src/lib.rs:243-259`), ends the session
  marker, and exits.
- `SIGTERM` does the same, immediately.
- **The daemon going away under a frontend** (it crashed, was killed, or
  stopped on `SIGTERM` with clients still connected) is learned from the
  socket, not from the next call: when the connection's reader ends, every
  pending call is answered `Disconnected` and then `Client::closed()`
  resolves. The frontend says so and keeps what is on screen, to read:
  the terminal's status line says "Postio's background service stopped —
  press R to reconnect" (refresh's key; `F5` too, which a composer does not
  type), and the desktop replaces the window's content with the unavailable
  screen, whose "Try again" reruns startup's connect-and-follow path.
- **Reconnecting is only ever asked for.** It runs `connect_or_start`
  again, starting a daemon if nothing answers, off the frontend's loop, and
  `Client::reconnect` swaps the new connection in under every clone of the
  client, so nothing holding one needs handing a new one; events and
  notifications are re-subscribed on `Client::reconnected`. What is on
  screen is then read afresh: the sidebar, the list where it was, the
  reader. The new connection is a new client to the daemon: its draft
  writer does not know earlier compositions by generation, so a draft
  carries the id its first save returned.
- **Nothing is replayed.** A command or a draft write made while the daemon
  was gone was answered `Disconnected` and is not retried. The terminal's
  composer keeps its text and saves it once reconnected; closing, sending
  or quitting with an unsaved draft while the daemon is gone is held (a
  quit asks once). A send the daemon had already queued is in its store,
  and its engine carries it on when it next runs.

## Observability

The daemon logs through `POSTIO_LOG`, like everything else. Per connection it
logs the client kind and id, and per request the family, id, duration and
outcome, **never an argument**: queries, addresses and bodies are content.

## Counting

`postio-client` counts round trips per `Req` family. Budget tests assert, for
example, that a list keystroke costs at most one `Page` and zero `Body`
requests.
