# Contract: frontend ↔ host

`postio-client` is what every frontend holds, and `postio-host` answers it,
inside the same process: the desktop app, the terminal app and the macOS app
each run their own host over the store they opened (ADR 0041). This is the
contract between the two halves.

> **Revised 2026-09-24.** This contract first carried the same requests over
> a Unix socket to `postio-daemon`, with a handshake, framing and a lifetime
> of its own. The daemon was withdrawn (spec Clarifications, 2026-09-24); the
> requests below are what survived it.

## Transport

In-process only. A call is posted to the host's runtime and answered over a
oneshot, with no encoding; events arrive on a channel in order. Every `Req`
is answered from local state. **No `Req` waits on the network** (Principle
I). Anything remote is a `Command`, which is enqueued, and its outcome arrives
later as `Event`s.

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

`Command::Undo` undoes the top of the one frontend's stack.

### Notifications

The host decides with `postio_ui::notify::decide`, unchanged, and hands the
decision to the frontend that runs it, which shows it: the desktop through
`gio::Notification`, the terminal on its status line and through
`notify-send` where a session bus is reachable. The folder gate is `[sync]
notify` and `notify_roles`.

## Lifetime

- The frontend opens the store, starts the host over it, and connects.
- **A store another app has open is refused**, before anything is read or
  written: the frontend says "Postio is already open in another window.
  Close it to open Postio here." The desktop shows it on its "cannot open"
  screen with a retry; the terminal prints it and exits.
- When the frontend quits, the host stops the engines and ends the session
  marker, and the store is free for the other app.

## Observability

The host logs through `POSTIO_LOG`, like everything else. Per request it logs the family, id, duration and
outcome, **never an argument**: queries, addresses and bodies are content.

## Counting

`postio-client` counts round trips per `Req` family. Budget tests assert, for
example, that a list keystroke costs at most one `Page` and zero `Body`
requests.
