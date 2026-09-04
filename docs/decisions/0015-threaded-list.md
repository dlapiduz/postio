# ADR 0015 — One row per thread, and the conversation pane

- **Status:** Accepted — **GO** (2026-08-25), **Q1 implementation and Q4
  revised 2026-08-26**, **Q4's column superseded 2026-09-03** (see §Q4)
- **Date:** 2026-08-25
- **Issue:** [#134](https://github.com/dlapiduz/postio/issues/134), decided by
  the maintainer: a single row per thread in the list, and the reading pane
  shows every message of the conversation — like Gmail.
  [#303](https://github.com/dlapiduz/postio/issues/303) tracked writing this.
- **Related:** the design canvas (3a's first bullet is one row per thread),
  #44 (the drill-in already reads the whole thread across folders — the
  precedent this builds on), `ARCHITECTURE.md` §4 (the list is windowed,
  selection is a predicate) and §6 (mailboxes are real, views are queries),
  ADR 0005 Q2 (`ThreadGroup` for the unified inbox composes on top of this).
- **Decision:** **folders thread; query views list messages.** The collapse
  is store-side — a windowed list over `threads` joined to its
  representative message — never a view-side grouping. A thread row carries
  the thread's newest activity and its aggregate state; every message verb
  on a thread row acts on the thread. The conversation pane shows all
  messages with read ones collapsed, and the first unread in focus.

> **Revised, and what changed.** Building Q1 and Q2 (#306, #307) turned up one
> thing the ADR got wrong, and building Q4 (#308) needed answers it had not
> given. Both are recorded below rather than quietly diverged from.
>
> 1. **Q1's window is over messages, not over `threads`.** As written it hides
>    mail — `messages.thread_id` is nullable and nothing guarantees it is set.
>    The decision the ADR was actually making (store-side collapse, flat
>    paging, one row per conversation) is unchanged. Now §Q1a.
> 2. **Q4 described a surface that could not coexist with the drill-in.** The
>    reading pane becoming a stack *and* `t` still opening a thread column
>    meant the same conversation listed twice, side by side. The column
>    survives with a different job: it is an **index**, not a second copy.
>    Now §Q4.
> 3. **Focus opens on the first unread, not on the newest.** Q4 said the newest
>    message expands. Opening a conversation should put you where you stopped
>    reading. Now §Q4.
> 4. **`a` never means one message.** Q4 left open that `a` "may later grow a
>    per-message meaning" in the reader. It does not: reply, reply-all and
>    forward are per message, and every other verb is the conversation's.
>    Now §Q4.

---

## The line that makes the rest fall out

`ARCHITECTURE.md` §6 already distinguishes two kinds of thing that share
the sidebar: **real mailboxes** (server state; mail lives there; `a`
archives *into* one) and **views** (queries re-run on open). This ADR
draws the same line through the list:

> **A real folder shows threads. A query view shows messages.**

- **Folders** (Inbox, Archive, a project folder) are where conversations
  live, so they show conversations: one row per thread, no
  `[ui] threaded` toggle. The canvas says threaded is the default; the
  maintainer's decision says it plainly; a mode would mean every list
  behaviour ships twice and is tested twice for an audience nobody has
  named.
- **Query views** — search results, and the Flagged smart folder — answer
  *"which messages match"*, and their rows stay messages. Search relevance
  is ranked per message and a hit deserves to be seen in its own words;
  Flagged shows the messages you starred, not conversations that happen
  to contain one. This is also the honest v1 cut: it changes nothing
  about the surfaces that already work.

## Q1 — The collapse is a query over `threads`

The issue's own analysis holds: collapsing in the view means reading rows
to throw them away, which breaks flat paging and the windowed-list
invariant. Instead the list becomes a window over **threads**:

- One page = N threads, ordered by `threads.last_at DESC` —
  `idx_threads_account_last_at (account_id, last_at DESC, id DESC)`
  already exists for exactly this shape.
- Each row joins the thread's **newest message in the current folder** as
  its representative (sender, snippet, attachment chip), plus aggregates:
  total message count, unread count, any-flagged. The aggregates come
  from the counted columns the mailbox-counts machinery already
  maintains where they exist, and a scoped `count(*)` where they do not —
  the bench (`store_reads`) is the referee, and `the_message_list_plan_never_sorts`
  keeps the plan honest.
- Paging stays flat: page k of threads costs what page k of messages
  cost, by the same argument, over the same kind of index.

## Q1a — The window is over messages (revised 2026-08-26)

Q1 says the list "becomes a window over **threads**". Implementing it that way
(#306) is wrong in a way that loses mail, so the shipped window is over
**messages**: the row kept is the one that is newest in its own thread within
the folder.

```sql
FROM messages rep
WHERE rep.mailbox_id = ?1 AND rep.deleted_locally = 0
  AND NOT EXISTS (SELECT 1 FROM messages newer
                   WHERE newer.mailbox_id = ?1 AND newer.deleted_locally = 0
                     AND newer.thread_id IS NOT NULL
                     AND newer.thread_id = rep.thread_id
                     AND (newer.received_at, newer.id) > (rep.received_at, rep.id))
ORDER BY rep.received_at DESC, rep.id DESC
```

**Why.** `messages.thread_id` is nullable, and nothing guarantees it is set —
`postio-sync`'s send path threads with a discarded result. A window built over
`threads` makes every unthreaded message *absent*: no error, no empty state,
mail in the store and not on screen. Under this shape an unthreaded message is
a conversation of one and cannot disappear, which is why
`ThreadListRow::id` is an `Option<ThreadId>`.

It is also flat **by construction** rather than by measurement: the window
walks `idx_messages_list (mailbox_id, received_at DESC, id DESC)`, the same
index the message list uses, so "page k of threads costs what page k of
messages costs" is the same query plan rather than a claim to benchmark.
Everything the conversation contributes — total size, unread here, flagged
here — is a correlated subquery per row of the page over
`idx_messages_thread_mailbox` (migration 0012). Measured: 897µs at 1k, 1.07ms
at 100k, 1.09ms ten pages down.

Every property Q1 decided survives. What changed is the table walked, so the
falsifiability lever below is spent: nothing needs denormalising.

**The account-scoped list still windows over `threads`**, because there is no
folder to be newest within.

**Drafts does not thread.** Q1 did not have to say so because it was writing
about reading mail. A draft is a document you are writing; two drafts
answering one conversation would collapse into a single row with no way to
reach the other. `SqliteStore::lists_conversations` is the only place this is
decided, so no frontend holds a second opinion.

## Q2 — What a collapsed row shows

Gmail's answers, adopted deliberately rather than re-derived:

- **Participants, not one sender**: the distinct senders of the thread,
  newest-biased, elided ("Ada, Grace 6" — the count is the thread's total
  size, which is what the badge already meant on the canvas).
- **Subject** is the thread subject (the normalised root subject the
  threading already computes).
- **Timestamp** is `threads.last_at` — the conversation's recency, which
  is what the sort key already is.
- **Unread** when any message *in this folder's slice* of the thread is
  unread — unread is what you act on from here, and a thread whose only
  unread member is in another folder still reads as handled in this one.
- The cursor key hints, the accent bar and the density metrics carry over
  from the message row unchanged; this is a content change, not a new
  row species.

## Q3 — The keys: a row is a thread

On a thread row, the message verbs act on the thread — `a` archives the
conversation, `d` trashes it, `s` flags it (the representative message
carries the star, matching what Gmail does), `U` marks the conversation
unread. This is Gmail's contract and it is what the row *is*: acting on
"the row" and acting on "one message of six" cannot both be what a key
means.

- `A` (archive thread) becomes synonymous with `a` **in the list
  context** and keeps its distinct meaning in the reader, where the
  cursor may sit on one message of an open conversation and `a` may
  later grow a per-message meaning there. The registry rows do not
  change; what changes is that the list's target resolution hands the
  thread's message set to the same verbs.
- **Selection stays a predicate.** Selecting thread rows selects
  threads; `Selection::Everything { except }` excepts thread ids; the
  *store* expands threads to member messages inside the verbs, exactly
  where bulk sets already expand. The frontend never materialises a
  conversation to act on it.
- Undo takes the whole unit back, which the `UndoStack` already handles
  — an archive of a six-message thread is one entry carrying its inverse.

## Q4 — The conversation pane, and what the column is for (revised 2026-08-26, **superseded 2026-09-03**)

> **Superseded, and it is worth saying why the compromise below did not hold.**
> Canvas turn 8 (#1000, #1003) removes the drill-in column outright: the list
> column is only ever the list, and the conversation lives in the reading pane
> and nowhere else.
>
> The "column is an index, pane is the conversation" split below was an honest
> attempt to keep a surface that already worked. What it bought was a table of
> contents; what it cost was **two surfaces holding the same conversation and
> a guarded echo between them** — the pane announcing focus, the column moving
> its cursor, the column announcing, the pane focusing, with a re-entrancy
> flag held for the duration of each call to stop it ringing. That is a lot of
> machinery to keep two lists of the same eight messages agreeing, and the
> only thing it enabled was jumping, which `J`/`K` in the pane now does
> without a second surface to keep in step.
>
> Everything else in this section stands and is now the pane's alone: the
> whole conversation across folders, read messages collapsed, one current
> message, focus on the first unread, `a` never meaning one message. `t`,
> `n` (unread-only) and `o` (order) are gone with the column, and
> `Context::Thread` is `Context::Conversation` — the surface it names is the
> pane.


Opening a thread row shows **the whole conversation, across folders** — which
is what #44 already made the drill-in read; this ADR gives it the Gmail shape
rather than a new scope.

The first version of this section could not be built as written. It made the
reading pane a stack of messages *and* kept `t` opening the thread column,
which is the same conversation listed twice, side by side. Resolved by giving
the two surfaces different jobs:

> **The column is an index. The pane is the conversation.**

- **The drill-in column** stays exactly where it is, one compact line per
  message, and stops driving what the reading pane *shows*. Its job is to
  **jump to a point in the conversation**: moving its cursor scrolls the pane
  to that message and expands it if it was collapsed. A table of contents,
  not a second copy. This repoints #436's wiring rather than deleting it —
  the column's cursor still drives the pane, it just scrolls it instead of
  replacing its contents.
- **The reading pane** holds every message of the conversation, oldest first,
  **read messages collapsed** to a one-line header (sender, snippet, date).
  Each expanded message is the existing reader surface — the hardened WebKit
  view, the parts panel, the remote-image banner — unchanged. A stack of the
  reader Postio already has, not a new renderer.

### Focus

There is **one current message**, shown in both surfaces: the column's cursor
and the pane's focused message are the same state. Moving either moves both.

- Opening a conversation focuses **the first unread message**, expanded and
  scrolled to. Not the newest: a conversation you open is one you are part
  way through, and landing at the end means scrolling back past what you have
  already read. When everything has been read there is no first unread, and
  focus lands on the **newest**, expanded.
- Focus is **drawn, not implied**. It uses the vocabulary PLATE 1b already
  established for the list — accent edge, full-strength ink — rather than
  dimming the messages around it. This is the one surface in the application
  where prose is actually read, and dimmed body text is a contrast and
  text-scaling problem before it is a focus treatment. Collapse already does
  the de-emphasising: a read message is one line. If two adjacent expanded
  messages still read flat, the answer is a background lift on the focused
  one, never dimming the other.

### The verbs

**Reply, reply-all and forward are per message.** They are drawn on each
message and act on the message they are drawn on, because answering the wrong
message of a conversation is a real and common mistake.

**Every other verb is the conversation's**, in the pane exactly as in the
list: `a` archives the thread, `d` trashes it, `s` flags it, `U` marks it
unread — wherever the cursor happens to be. One key, one meaning, in both
places. This replaces the earlier note that `a` "may later grow a per-message
meaning" in the reader: it does not.

### Cost, which is what decides the shape

Every expanded message is a `WebKitWebView`. "Unread and newest expanded" over
a thirty-message conversation nobody has read is thirty of them, which holds
neither the interaction budget nor the memory.

So readers are **instantiated lazily as messages scroll into view**, and eager
expansion is **capped** — the focused message and a small number after it —
with the rest collapsed and one keystroke or click from opening. The budget
may be stretched for a genuine outlier; the design does not bend around one.

### Reading

Read marking stays per message on #71's dwell rules, driven by **focus**
rather than by raw scroll position: the focused message is the one being read,
and dwelling on it marks it. Never "opened the thread, all six read".

## Q5 — What this deliberately does not decide

- **The unified inbox's cross-account grouping** stays ADR 0005 Q2's
  `ThreadGroup`, which composes on top of thread rows (a group is one or
  more of these rows' threads); #184 builds it.
- **Search results as conversations** (Gmail does collapse them) is a
  recorded v1 cut, not a principle. If message-rows-in-search reads as
  inconsistent in use, that is a new decision with this ADR as its
  context.

## What would falsify this

The Q1 bet is that thread-paged reads stay flat and inside the budgets
with the aggregate columns doing the counting. If `store_reads` shows the
join breaking the 16 ms interaction budget at 100k messages, the fallback
is denormalising the row's aggregates onto `threads` (maintained where
counts are already maintained), not view-side collapsing — the windowed
invariant is not on the table.

**Settled 2026-08-26 (#306):** the bet held, by a wide margin and without the
fallback — 1.07ms at 100k against a 16ms budget, flat with depth. See §Q1a for
the shape that produced it.

The open bet is now Q4's: that a stack of lazily-instantiated `WebKitWebView`s
stays inside the interaction budget for an ordinary conversation. If it does
not, the fallback is a **single** reader that re-renders as focus moves —
which is what the pane does today — with the collapse/expand list around it,
rather than giving up the conversation view.

---

## Consequences

- `postio-storage` gains the thread-window query beside the message one;
  the repository keeps both, because query views still page messages.
- `postio-gtk`'s list feeds thread rows in folder scopes; the row widget
  gains participants/aggregate content; the reading pane becomes a
  message stack with collapse state, and the drill-in column becomes its
  index rather than a second copy of it.
- `postio-core`: list-context target resolution maps thread rows onto the
  existing verbs; no registry changes.
- Implementation lands as three sequenced issues under E7 — the store
  query, the list row, the conversation pane — filed with this ADR.
