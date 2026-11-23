# ADR 0015 — One row per thread, and the conversation pane

- **Status:** Accepted (2026-08-25); revised 2026-09-09 (open on the newest message; the document pane shows every body). The drill-in column was removed (canvas turn 8). The desktop app shows one message at a time; the terminal and macOS draw a thread as one document ([ADR 0032](0032-the-conversation-is-one-document.md)).
- **Decision:** folders thread; query views list messages. The collapse is store-side, never a view-side grouping. A thread row carries the newest activity and aggregate state, and every message verb on it acts on the thread.

## Rules
- A real folder shows one row per thread, with no `[ui] threaded` toggle. Search results and the Flagged view stay message rows. Drafts never thread (`LocalStore::lists_conversations` is the one place that decides).
- The folder window is over messages: keep the row that is newest in its own thread within the folder (`NOT EXISTS` a newer message with the same `thread_id`), walking `idx_messages_list`. `messages.thread_id` is nullable: an unthreaded message is a conversation of one, so `ThreadListRow::id` is `Option<ThreadId>` and mail never disappears. The account-scoped list windows over `threads`.
- Aggregates (total, unread here, flagged here) are per-page correlated subqueries over `idx_messages_thread_mailbox`. The `store_reads` bench and `the_message_list_plan_never_sorts` gate it (1.07 ms at 100k).
- A row shows: distinct participants, newest-biased and elided, with the thread's total count; the normalised thread subject; `threads.last_at`; unread when any message in this folder's slice is unread.
- On a thread row `a` archives, `d` trashes, `s` flags and `U` marks unread the conversation. `A` equals `a` in the list. Selection stays a predicate over thread ids; the store expands threads to messages inside the verbs. Undo takes the whole unit back.
- The conversation pane shows the whole conversation across folders; the pane is the only surface for it. Opening focuses the newest message. Reply, reply-all and forward act on the message they are drawn on; every other verb is the conversation's, wherever the cursor is. Focus is drawn (accent edge, full-strength ink), never by dimming others. Read marking is per message, driven by focus and dwell, never "opened the thread, all read".
- A stacked pane (one view per message) instantiates readers lazily and caps eager expansion (`EAGER_EXPANSION_CAP`). The one-document pane has no collapsing, because one document is flat in memory (~101 MiB, 47-102 ms regardless of message count). A conversation of a hundred or more messages must not prepare every body at once (FR-051).
- The unified inbox groups with `ThreadGroup` (ADR 0005 Q2) on top of these rows.

## Why
- View-side collapse reads rows to throw them away and breaks flat paging. A window over `threads` hides unthreaded mail silently.
- A mode toggle ships every list behaviour twice. A row means one thing, so a key acts on the row.
- A column mirroring the pane needed a guarded echo between two copies of one conversation.

## Revisit when
- Search results should collapse into conversations: a new decision with this one as context.
