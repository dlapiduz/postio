# Research: Threads in Postio Focus, Mac and Linux

Each entry: what the code does today (read on 2026-11-11 at `d3969176c`),
the decision, and what was rejected.

## R1. Where thread state lives today

**Found.**
- `threads` (`postio-storage/src/schema.rs`) holds aggregates only: subject,
  counts, `has_attachments`, `is_flagged`, `first_at`, `last_at`.
  `thread_links` maps each claimed folded `Message-ID` to a thread.
- Snooze is per message (`messages.snoozed_until`), labels per message
  (`message_labels`), folder per message (`mailbox_id`).
- Reminders are already per thread (`reminders.thread_id`). The schema says
  why `thread_id` is not a foreign key: **a threading pass may renumber
  conversations.**
- Focus rows already aim verbs at the whole conversation:
  `postio_ui::focus_target::aim_by` sends `MessageTarget::Threads(row.threads)`,
  and `dispatch` maps `ArchiveThread` to `Archive` ("a Focus row is a whole
  conversation"). Archive, delete, read and flag on a row reach every message
  of the thread.

**Decision.** D3's "thread state" is the existing write-through: an action
on the thread writes every member message (`MessageTarget::Threads`), as
Focus rows do now. Nothing moves from messages to threads in the schema.
Only the genuinely new per-thread state gets storage: mute and split (R3).
Reminders stay as they are.

**Rejected.** Columns on `threads` for snooze, labels and folder: they would
duplicate what the server keeps on each message. Thread ids are also not
stable across a rethread, so state keyed by them would be lost.

## R2. Grouping: subject window, list mail, and the merge the fallback causes

**Found.** `postio_model::threading::assign` tries references first, nearest
first. Only when none links does it fall back to
`ThreadIndex::threads_with_subject`, and only when the subject carries reply
decoration. There is no date window and no list exclusion. When the fallback
finds several threads with the same subject, it **merges all of them**
(`found.len() > 1` → `Assignment::Merge`), so every "Re: Hello" ever
received collapses into one thread.

**Decision.**
- The subject fallback returns at most one thread: the newest whose
  `last_at` is within 7 days of the message's date. It **joins** that
  thread and never merges. `ThreadCue` gains `date` and `is_list`.
- `is_list` comes from `Message::list_id` plus the bulk test the filing pass
  already uses (`is:bulk`). When it is set, the fallback is skipped.
- `ThreadIndex::threads_with_subject` gains a `since` bound. The storage side
  adds `last_at >= ?` to the query it already makes, on an index over
  `(account_id, subject, last_at)`.
- Existing stores are not rethreaded by a migration. New mail follows the
  rule, and `rethread_orphans` already re-asks for messages that had no
  answer. There are no installs to protect (CLAUDE.md), so a resync is the
  fix for a store grouped by the old rule.

**Rejected.** A full rethread migration: it would scan the mailbox (against
constitution V) to fix a handful of over-merged subjects.

## R3. Mute and split, keyed by what survives a rethread

**Found.** Thread ids are renumbered by merges and by a resync. `thread_links`
keys on `(account_id, folded rfc_message_id)`, and that key is stable.

**Decision.**
- **Mute**: a table `thread_mutes (account_id, rfc_message_id, muted_at,
  replies_since)`, where `rfc_message_id` is the folded id of the thread's
  root when muted. A thread is muted when any of its claims is in
  `thread_mutes`, a join through `thread_links` that stays true across
  merges and renumbering. Unmute deletes the row. `replies_since` is bumped
  by the filing stage (R4).
- **Split**: a table `thread_splits (account_id, rfc_message_id, split_at)`
  naming the split message's folded id.
  - `assign` gains one rule: when the links found point at more than one
    thread and one of them is a split thread, the message **joins the thread
    of its nearest link** and nothing merges.
  - Splitting reassigns the message and its descendants to a new thread.
    Descendants are found in memory over the thread's members by
    `In-Reply-To`/`References`, which is O(thread). Their claims are relinked
    and both threads' aggregates are recomputed.
- Both are local. They do not sync across machines (spec Assumptions); their
  effects (archived replies) do.

**Rejected.**
- A `muted` column on `threads`: lost on the renumbering the schema already
  warns about.
- An IMAP keyword for mute: it would sync, but there is no standard keyword,
  and Gmail's mute is not exposed over IMAP. That can come later, the way
  #2076 did it for Filtered.

## R4. Mute on arrival: Focus's filing stage

**Found.** Focus classifies mail as it is filed (`FocusStage::Filing`,
`postio-storage/src/repository/focus_classified.rs`). It files some mail out
of the inbox into the Archive with a reason (`filter_decisions`) and a server
keyword (#2076).

**Decision.** The filing stage checks whether the message's thread is muted
before it applies any other rule. If it is, the stage archives the message
the same way any archive does: an ordinary move the server sees. It bumps
`thread_mutes.replies_since` and emits the thread changed. That costs one
indexed lookup per filed message, through `thread_links` on the message's
own links, which the threading pass has just written.

A muted reply is **not** a Filtered decision: Filtered is "Focus judged this
mail", and mute is the person's own instruction.

**Rejected.** A seventh `FilterReason` "muted": it would show muted replies
in Filtered, which the design does not ask for and which would double-count.

## R5. A message's own line, without fetching

**Found.** `messages.preview` is cut from the whole body text, quotes
included (`postio-model/src/mime.rs`, `preview`). `postio_body::own_text`
gives the sender's own words without quoted history or signature. Focus
already computes it once the body is local (`FocusStage::Body`, the
needs-action detector, `postio-host/src/focus/rules.rs`).

**Decision.**
- A column `messages.own_line TEXT`, written by the body stage from
  `own_text` and cut to the preview's length.
- Child rows show `own_line`. When it is null (no body yet), they fall back
  to `preview` with the quote cut at the first attribution line
  (`quote::without_attribution`), and to a placeholder when that is empty.
- An all-quote message has `own_line = ''`, which draws "Quoted reply only".
- Nothing is fetched to draw a row (FR-028).

**Rejected.** Computing own text when the row is drawn: it parses HTML on the
main thread per child row, which breaks the 16 ms budget.

## R6. Reading a thread: one request, one statement

**Found.** `Client::conversation(thread)` returns `Vec<MessageSummary>`,
oldest first. It has no `in_reply_to`, own line or draft, and no
`rfc_message_id` to resolve "↳ to Ana" with. The Mac
(`postio-ffi/src/focus_message.rs`) and GTK (`open.rs`) both call it to step
through a thread.

**Decision.** A new request, `Req::Thread(ThreadId)`, answers a
`ThreadView` (data-model): every member in date order with sender, date,
seen, attachments, own line, `answers` (the member index its `In-Reply-To`
names, when that is not the previous member), and a trailing draft from
`drafts.thread_id`. It is one statement over `idx_messages_thread` plus one
over `idx_drafts_thread`, gated as a count. `answers` is resolved in memory
over the members' `rfc_message_id`/`in_reply_to`. `conversation` is retired
once both frontends use `Thread` (no backwards compatibility).

Long threads: the read is O(thread) and bounded by the thread, not the
mailbox. The frontends window the child rows and the outline. The bar
computes which chips fit without drawing the rest (R8).

## R7. The list: expansion as a splice over the windowed rows

**Found.** The controller never holds rows. It asks the frontend through
`postio_focus::cursor::Rows` (`len`, `facts(position)`), and the rows are
paged per toolkit.

**Decision.**
- The controller holds one `Expansion { thread, anchor: MessageId,
  members: ThreadView }` at most.
- `postio_ui::thread_list::Splice` maps a display position to a `Slot`:
  `Row(position)` or `Child { row, index }`. The anchor row's children are
  inserted after it, and `len` grows by their count.
- Both frontends draw through the splice: AppKit
  `insertRows`/`removeRows` on `FocusListTable`, GTK a splice on the list
  model.
- The cursor, `j`/`k`, scope and footer all ask the splice, so one set of
  unit tests covers both platforms.
- An expansion whose anchor leaves the list collapses. One whose thread
  gains a member re-reads the `ThreadView` and keeps the focused child by
  `MessageId`.

**Rejected.** Children as real rows in the paged store query: it would change
every page's offsets and cursor arithmetic, and would put a per-row thread
read into the hot list query.

## R8. The thread bar: which chips fit

**Decision.** A pure function in `postio_ui::thread_bar`,
`chips(members, current, unread, matches, room) -> Vec<Chip>`, where `room`
is how many 72-point chips fit. When everything fits, every member is shown.
Otherwise it keeps the first, then "… N more", then the latest that fit, and
always keeps the current and every unread chip. When the forced chips do not
fit either, several "… N more" chips stand between kept runs. The kicker,
banners and Reply to row are word functions in the same module, so the Mac
and GTK draw identical text.

## R9. Scope: one function

**Decision.** `postio_ui::scope::target(focus: ScopeFocus, action: CommandId)
-> Scope`, where `ScopeFocus` is `Row { thread, latest } | Child { thread,
message } | Window { thread, message }` and
`Scope = Thread(ThreadId) | Message(MessageId) | Unavailable`. It encodes
D2's table cell by cell and is the only place the table lives.
`focus_target::aim_by` uses it for the cursor's row, and the window's verbs
use it for the shown message. Selections (`x`) stay whole-thread, as today.

Change against today: **flag** and **read/unread** on a collapsed row. Flag
now goes to the latest message, and read/unread still goes to the whole
thread (D2).

## R10. Keys and contexts

**Found.** The registry's contexts are List, Conversation, Reader, Search,
Results, Composer, Picker and others. `ReaderVerb::StepThread(i32)` is
already routed by `postio-focus/src/surfaces.rs` from
`NextInConversation`/`PrevInConversation` (`n`/`p`, `alt+Down`/`alt+Up`).
`[`/`]` are bound only in search results (matches), the digest (references)
and contacts (tabs). This was checked against `docs/keybindings.md`.

**Decision.** New `CommandId`s, each with a registry test that it does not
clash in its contexts:

| Command | Keys | Contexts |
|---|---|---|
| `expand_thread` / `collapse_thread` | `Right` / `Left` | List, Results, Filtered view |
| `prev_in_conversation` / `next_in_conversation` (existing) | gain `[` / `]` as default | Reader |
| `first_in_conversation` / `latest_in_conversation` | `{` / `}` | Reader |
| `first_unread_in_conversation` | `g u` (Reader); `u` (Outline) | Reader, Outline |
| `thread_outline` | `alt+t` | Reader, List |
| `thread_menu` | none (the button), offered in the palette | Reader |
| `reply_to_latest` | `alt+e` | List, Reader, Results, Composer (not the terminal) |
| `mute_thread` | `m` | List, Reader, Results |
| `split_thread` | none, menu only | Reader |
| `move_this_message` / `delete_this_message` | `alt+v` / `alt+BackSpace` | Reader, and child rows in List and Results |
| `forward_what_1..3` | `1` `2` `3` | Forward chooser |
| `show_newer` / `expand_quoted` | `mod+shift+Down` / `mod+shift+q` | Composer |

- `Right`/`Left` are removed from `open_message` and `prev_view` in List,
  Results and Filtered view.
- `archive_thread` (`E`) is deleted.
- `find_next`/`find_previous` keep their keys. In a window opened from
  search they step through `ThreadMatches` (R11).

## R11. Search: children and stepping between matches

**Found.** `Client::conversations` answers `ConversationHit`s with a match
count. `Client::conversation_matches(query, key)` answers every match in one
conversation, oldest first, with its `Match` (source and passage). That is
Quick Look's read (#2085, #2098).

**Decision.**
- The results' expanded thread reads `conversation_matches` once for the
  thread. Its children are the matched messages. Runs of members between
  them, taken from the `ThreadView`, become "Message N doesn't match" lines
  with sender and day.
- The top multi-match result's matches are read with the page, so its
  default expansion costs no extra round trip on screen.
- Opening a child carries `ThreadMatches { query, messages, at }` into the
  window. `find_next`/`find_previous` first step within the shown message's
  find matches, then on to the next matched message.
- `Escape` returns to Results with the cursor on that child, through the
  results' History (spec 010).

## R12. Compose

**Found.** `Draft` has `in_reply_to: Option<MessageId>` and `thread_id`.
Reply builds the quote from the message replied to (`postio-body/src/replying.rs`).

**Decision.**
- **Reply to latest**: a composer command that swaps `in_reply_to`, the
  quote and the threading headers to the thread's latest member, and keeps
  the typed text. The draft's text above the quote is preserved by the
  composer's existing quote split.
- **Older-reply bar**: shown when `in_reply_to` is not the latest received
  member of the `ThreadView`.
- **Forward chooser**: a composer mode with `ForwardForm::{This, Thread,
  Attachments}`.
  - `Thread` builds one body in `postio-body` (`forward_thread`): each member
    oldest first under a "From · date" header, with its own text and quote
    folded as the reader folds it, plus the attachments of every member,
    deduplicated by content hash.
  - `Attachments` attaches each member's raw source as `message/rfc822`
    (`.eml`), from the local raw store.
  - **Members whose body or raw source is not local block that form** with a
    notice. Choosing it does not fetch (constitution I and VI).
  - The subject is `Fwd: <subject> (N messages)`.

## R13. Platforms and verification

**Found.** This Mac cannot build `postio-gtk` (spec 010 plan). The Linux box
takes at most two agents (maintainer's note, 2026-10-23). CI's `Frontends`
check runs every GTK suite on the PR.

**Decision.**
- Every slice ships its shared Rust and both frontends in one commit series.
- GTK is verified by the `focus_suite` cases and storyboards on CI, and by
  screens shot on the Linux box (`scripts/run-isolated.sh --shot`), one agent
  at a time.
- The Mac is verified with `scripts/macos-test.sh` and `scripts/macos-shot.sh`
  at 1440×900.
- Storyboards are toolkit-neutral (`storyboards/threads/`) and are written
  red first.

## R14. Demo store

**Decision.** `Seed::Threads` in `postio-demo`. It holds the design's
conversations with invented names on reserved domains: "Harbor API draft
v3" with six messages, two of them unread, a reply to message 1, a file and
a draft; a 23-message thread; a muted 41-message list thread; single
messages; and the search fixtures for screens 06 and 07. It is seeded
through `ThreadingRepository`, so grouping is the real rule, not a fixture's
claim.
