# ADR 0015 — One row per thread, and the conversation pane

- **Status:** Accepted — **GO** (2026-08-25)
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
  messages with read ones collapsed, unread and newest expanded.

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

## Q4 — The conversation pane

Opening a thread row shows **the whole conversation, across folders** —
which is what #44 already made the drill-in read; this ADR gives it the
Gmail shape rather than a new scope:

- All messages, oldest first, **read messages collapsed** to a one-line
  header (sender, snippet, date), **unread messages and the newest
  message expanded**. Expanding a collapsed message is a click or the
  cursor plus `Enter`; `t` keeps toggling the drill-in as a whole.
- Each expanded message is the existing reader surface — the hardened
  WebKit view, the parts panel, the remote-image banner all apply per
  message, unchanged. The conversation pane is a stack of the reader
  Postio already has, not a new renderer.
- Reading a collapsed-by-default conversation marks messages read on the
  same dwell rules as today (#71's semantics), per message as they are
  expanded or scrolled through — never "opened the thread, all six read".

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

---

## Consequences

- `postio-storage` gains the thread-window query beside the message one;
  the repository keeps both, because query views still page messages.
- `postio-gtk`'s list feeds thread rows in folder scopes; the row widget
  gains participants/aggregate content; the reading pane becomes a
  message stack with collapse state.
- `postio-core`: list-context target resolution maps thread rows onto the
  existing verbs; no registry changes.
- Implementation lands as three sequenced issues under E7 — the store
  query, the list row, the conversation pane — filed with this ADR.
