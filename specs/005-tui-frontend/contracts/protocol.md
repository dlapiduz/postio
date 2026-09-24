# Contract: client ↔ daemon

`postio-client` is what every frontend holds. `postio-host` serves it, either
in-process or as `postio-daemon` over a socket. This is the contract between
them. It is private to one build: client and host always ship together, and a
mismatch is refused, never negotiated.

## Transport

- **Socket**: `$XDG_RUNTIME_DIR/postio/daemon.sock`, directory `0700`,
  socket `0600`. Never TCP. A connecting peer's uid is checked with
  `SO_PEERCRED` and must equal the daemon's.
- **Lock and pid**: `$XDG_RUNTIME_DIR/postio/daemon.lock`, held with `flock`
  for the daemon's lifetime. Turso's own file lock is the backstop.
- **Framing**: a length-prefixed frame (u32 big-endian) carrying a serde
  encoding of `Frame`. The encoding (`postcard` or `bincode`) is decided by
  the first task. It must round-trip every existing `Command` and `Event`,
  which already derive serde.
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

The API surface. Each maps to logic that exists today; the source is noted.

| Family | Requests | Moved from |
|---|---|---|
| Commands | `Send(Command)`, `SendTracked(Command)` → `Accepted \| Refused(reason)` | `postio_core::bridge::CommandSender` |
| Scopes and lists | `OpenScope(Scope)`, `Page { scope, offset, limit }`, `Count(scope)`, `Rows(ids)`, `Mailboxes`, `DraftCounts`, `ThreadPage`/`ThreadCount` | `MailStore` (`crates/postio-runtime/src/store/mod.rs:266`) |
| Reading | `Conversation(thread)`, `Body(message)` → `Body \| Reason`, `ResolveCid(message, cid)`, `Parts(message)`, `SavePart { part, path }`, `OpenPart(part)` → a temp path, `AllowRemoteImages(sender)`, `ActivateUnsubscribe(message)` | `postio-app/src/reading.rs`, `postio-session/src/reading.rs`, `postio-gtk/src/reader/allowlist.rs` |
| Search | `Search { query, scope, order }`, `Facets`, `Suggest(prefix)` | `postio-session/src/search.rs`, `postio-app/src/search.rs` |
| Compose | `NewDraft(kind: New\|Reply\|ReplyAll\|Forward, source)`, `SaveDraft(DraftBody)`, `DeleteDraft`, `QueueSend { draft, at: Option<Time> }`, `CancelSend`, `Identities`, `Signature(identity)`, `Recipients(prefix)`, `AttachFile(path)`, `AttachBytes { name, mime, bytes, inline: bool }` | `postio-app/src/compose.rs`, `postio-gtk/src/composer.rs:586-680` |
| Accounts | `Discover(address)`, `AddAccount(…)`, `BeginOAuth(account)` → `{ consent_url }`, `OAuthStatus`, `RemoveAccount`, `Credentials(…)` | `postio-app/src/onboarding.rs`, `settings_accounts.rs`, `settings_credential.rs` |
| Settings | `Settings`, `PatchSettings(patch)` | `postio-ffi/src/settings.rs`, `postio_ui::settings` |
| Status | `SyncStatus`, `Egress` | `postio_ui::status`, `settings_egress.rs` |

`DraftBody` carries `markdown: Option<String>`, `text`, `html:
Option<String>`, headers and attachment refs. The host stores what it is
given and does not re-derive it.

### Undo

`Command::Undo` from a client undoes the top of **that client's** stack
(research R1a). The `UndoAvailable`/`Undone` events are sent only to the
client that owns the entry. Every other event goes to every client.

### Notifications

The host decides with `postio_ui::notify::decide`, unchanged, then sends
`Notify` to exactly one client: the first connected `Gtk` client, otherwise
the first `Tui` client. With no clients there is no notification.

## Lifetime

- The daemon starts serving once the store is open. Before that, a `Hello`
  gets `Refused(Starting)` and the client retries.
- When its last client disconnects, it waits 30 s (research R1b). A new
  client cancels the wait. Otherwise it stops the engines (as `stop_retained`
  does today, `crates/postio-app/src/lib.rs:243-259`), ends the session
  marker, and exits.
- `SIGTERM` does the same, immediately.

## Observability

The daemon logs through `POSTIO_LOG`, like everything else. Per connection it
logs the client kind and id, and per request the family, id, duration and
outcome, **never an argument**: queries, addresses and bodies are content.

## Counting

`postio-client` counts round trips per `Req` family. Budget tests assert, for
example, that a list keystroke costs at most one `Page` and zero `Body`
requests.
