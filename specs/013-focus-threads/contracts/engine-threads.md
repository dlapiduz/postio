# Contract: the engine's thread requests

What `postio-focus`'s `perform` and the composer ask of `postio-client`, how
the host answers, and what each one costs. Types are in
[data-model.md](../data-model.md).

## New requests (`postio-client/src/protocol.rs`, `api.rs`)

| `Req` | `Resp` | `Client` method | Host → session/storage | Statements |
|---|---|---|---|---|
| `Thread(ThreadId)` | `Thread(Option<Box<ThreadView>>)` | `thread(id)` | `ThreadRepository::members` + `drafts` by `thread_id` + `thread_mutes` join; `answers` resolved in memory | ≤ 3, whatever the thread's length |
| `Mute(ThreadId)` / `Unmute(ThreadId)` | `Done` | `mute(id)` / `unmute(id)` | `thread_mutes` write; emits `ThreadChanged`; undoable | ≤ 2 |
| `Split(MessageId)` | `Split(ThreadId)` | `split(id)` | `ThreadRepository::split` in one transaction; emits `ThreadChanged` for both | O(thread) writes, no mailbox read |
| `MutedOf(Vec<ThreadId>)` | `Muted(Vec<(ThreadId, u32)>)` | `muted_of(ids)` | one statement per list page | 1 |
| `ForwardThread { thread, form }` | `Draft(DraftId)` | `forward_thread(..)` | `postio_body::forward_thread` or `forward_as_attachments` over local bodies/raw; refuses with `NotLocal(Vec<MessageId>)` | O(thread) |

`Req::family()` gains each name, so `postio-client/src/counting.rs` counts
them.

## Changed requests

- **`Conversation(ThreadId)`** is retired once both frontends read `Thread`.
  Its callers are `postio-ffi/src/focus_message.rs` and
  `postio-gtk/src/open.rs`.
- **Verbs with a `MessageTarget`**: there is no protocol change. Scope is
  resolved before the command is built (`postio_ui::scope::target`), so a
  child row's reply carries `Messages([id])` and a row's archive carries
  `Threads([..])`, as today.
- **List pages** (`FocusListQuery`): each `ThreadSummary` gains `muted:
  Option<u32>`, read in the page's existing statements plus at most one
  (`MutedOf`).
- **Composer**:
  - `ReplyToLatest { draft }` swaps `in_reply_to`, `References`, and the
    quote to the thread's latest received member, and keeps the text above
    the quote.
  - `SetForwardForm { draft, form }` rebuilds the forwarded content and keeps
    the text above it.

## Arrival (sync → Focus filing stage)

The order for each new message:

1. The message is threaded (`ThreadingRepository::thread`, with the R2 and
   R3 rules).
2. In the filing stage, if its thread is muted, it is archived like any
   archive, `replies_since` is incremented, and no other filing rule runs.
3. Otherwise, today's filing.
4. `ThreadChanged { thread, members_added }` is emitted after the
   transaction commits.

The body stage writes `messages.own_line` alongside the markers.

## Budgets (asserted with `postio_storage::test_support::counting`)

- `Thread` costs the same number of statements for a 2-message thread and a
  500-message one.
- A list page with mutes costs today's statements + 1.
- Threading a message with the subject fallback costs today's statements
  (the bounded index lookup replaces the unbounded one).
- The filing stage's mute check costs 1 statement per message.

## Logging

Ids, counts and outcomes only: `thread`, `members`, `muted`, `split_from`.
Never a subject, name, address or line of text (FR-029).
