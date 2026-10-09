# ADR 0021 — Sending is at-most-once, and an interrupted send is reported rather than guessed

- **Status:** Accepted (2026-08-28)
- **Date:** 2026-08-28
- **Decision by:** a `/ux-architect` session, on the question
  [#461](https://github.com/dlapiduz/postio/issues/461) raised behind
  [#423](https://github.com/dlapiduz/postio/issues/423): now that the Send
  button is actually wired, what stops a message going twice?
- **Issue:** [#461](https://github.com/dlapiduz/postio/issues/461)
- **Related:** [#423](https://github.com/dlapiduz/postio/issues/423) (wired
  Send), [#433](https://github.com/dlapiduz/postio/issues/433) (a queued draft
  is still editable), [#411](https://github.com/dlapiduz/postio/issues/411)
  (the status line shows numbers), ADR 0006 (credentials), `PRODUCT.md` §10,
  §14, §16, §21
- **Decision:** **Postio guarantees at-most-once submission and never
  guesses.** Three parts, in the order they matter: the `Message-ID` is minted
  once per send attempt series and stored on the draft; the durable commit
  point moves to the instant SMTP accepts, ahead of every network step that
  follows it; and an SMTP session that dies after the message payload has been
  submitted is **not retried** — it settles as a fifth drain outcome,
  `Uncertain`, and becomes a visible `Unconfirmed` draft that Postio then tries
  to resolve for the user by looking for its own `Message-ID` in the Sent
  mailbox.

---

## Why it needs deciding

Three facts about a naive send path make it duplicate, or lose track:

- **A retry that rebuilds the message mints a new `Message-ID`**, so a second
  delivery is a distinct message nobody downstream can recognise as a
  duplicate.
- **A connection that drops during `DATA` looks transient.** A payload
  written and then a lost reply to the terminating `.` is indistinguishable,
  by error kind alone, from a connection that died before `MAIL FROM`.
- **The durable fact that stops a resend must not sit behind network work.**
  If it is the deletion of the draft at the end of filing the Sent copy, an
  IMAP `APPEND` of the whole message sits in the crash window.

And the user has to be told: a send that fails permanently must leave a draft
in a state that says so, not a `Queued` draft that will never be retried.

## The three windows

| Window | Can the client know what happened? | Closable? |
|---|---|---|
| Before the payload is submitted — connect, auth, `MAIL FROM`, `RCPT TO` | Yes. The server has nothing. | Already closed. Retry is correct and safe. |
| Between submitting the payload and reading the final reply | **No. Not by any means SMTP offers.** | Not closable. Must be *decided*. |
| Between the final reply and the local record of it | Yes, if the record is written first. | Closable: Decision 2. |

The middle one is the whole difficulty. SMTP has no message-level identity the
way `UIDPLUS` gives IMAP `APPEND` a confirmable one; there is no idempotency
key to present on a second attempt, and nothing to ask the server afterwards.
The other two are engineering. This ADR closes both of those and makes an
explicit product decision about the one that cannot be closed.

## Decision 1 — one `Message-ID` per send attempt series

`drafts` gains an `rfc_message_id` column. `DraftRepository::queue_send` and
`queue_send_at` mint it in the same transaction that writes the
`Operation::Send` row, and `outgoing::build` takes it as a parameter rather
than generating one. Every attempt at the same queued draft carries the same
id.

It is **cleared when the draft returns to `Editing`**. That is the subtle half.
Reusing the id across an edit would be worse than not having one: a user who
was told a send is unconfirmed, opens the draft, fixes it and sends it again is
composing a *different message*, and a receiver that dedups on `Message-ID`
would silently drop the corrected version in favour of the one that may have
arrived. The id identifies one attempt series at one piece of text, not a row
in a table.

The point of this is **not** receiver-side dedup. That is a welcome side
effect on the systems that do it, unavailable on the many that do not, and not
something Postio may promise the user. The point is that Postio can recognise
its own message when it comes back — see Decision 3.

## Decision 2 — the commit point is the moment of acceptance

Two durable marks replace the accidental one:

1. **Immediately before `send_message`** — after the connection is open and
   authenticated, so that connect and auth failures stay ordinarily retryable —
   the draft goes to `DraftState::Sending`, committed.
2. **Immediately after `send_message` returns `Ok`**, in one store
   transaction and before `quit()`, the `APPEND`, or anything else, the draft
   goes to `DraftState::Sent` with the time it was accepted.

`send::resolve` then reads those states, and this is what makes them a
guarantee rather than a decoration:

- `Sent` → `ResolvedSend::Obsolete`. Never rebuilt, never resubmitted. The
  filing that did not finish is bookkeeping for a repair pass, not a reason to
  send.
- `Sending` on a fresh process → **`Uncertain`**, never resent. A draft found
  in `Sending` at startup is one whose process died with a connection open at
  the one point where the answer is unknowable.

That second rule is deliberately over-cautious: a crash between `MAIL FROM`
and the payload leaves a draft marked `Sending` that in fact never went, and
the user gets asked a question with a boring answer. That is the correct bias.
**Every ambiguity in this path resolves toward asking rather than
duplicating**, because the two mistakes are not symmetric — see Decision 3.

`file_sent_copy` is otherwise unchanged; it remains best-effort, and its
existing rule that nothing past acceptance may become `Failed` or `Retry`
stands. What changes is that its progress is no longer what the guarantee
rests on.

It also gives undo-send an end. `Operation::Send` has no inverse; undoing a
send is a *cancel against the queue* (`operation.rs`). With mark 1 in place,
the cancel is refused the moment the draft leaves `Queued`, so it can never
land on a row whose SMTP transaction is open, and `Recovery::Window` on
`CommandId::Send` is a claim about a window that actually has an end.

## Decision 3 — an indeterminate submission is reported, not retried

`SmtpError` gains a predicate beside `is_transient` and
`is_authentication_failure` — the crate's own docs require callers to branch on
predicates and never on variants, and this follows that rule:

```rust
/// Whether the message payload may already have reached the server.
pub fn submission_is_indeterminate(&self) -> bool
```

True for a `Disconnected`, `TimedOut`, `Io` or `Cancelled` failure raised once
the payload has begun being written, false for the same failures before it.
`postio-smtp` has to track that boundary inside `data`; the drainer checks this
predicate *before* `is_transient`, because a dropped connection is transient in
general and indeterminate here specifically.

`Outcome` has a fifth variant, `Uncertain { reason }`. `DrainReport::failed`
means *did not happen*, and the runtime turns it straight into
`Event::Error`. An interrupted
send may well have happened, so it must not travel as a failure and must not
travel as a success. `Uncertain` settles the queue row — done, with the reason
recorded, no further attempts — and surfaces separately in `DrainReport` and
`DrainSummary`.

**Postio then tries to answer the question without asking the user anything.**
The Sent mailbox is already flagged for resync in this path. On the next sync
of Sent, a message carrying the draft's reserved `Message-ID` means the send
went: the draft becomes `Sent`, the local copy is filed from what was fetched,
and the `Unconfirmed` banner is replaced by an ordinary "Sent 4 minutes ago".
Many submission servers file the sender's copy themselves, so for a good share
of users this resolves silently within one sync and they never learn anything
went wrong. Where the server does not file, the state stands and the user
decides.

That check makes no request the user did not ask for. Syncing the Sent mailbox
is ordinary sync of a folder the user already has; nothing new goes out, and no
third party learns anything (`PRODUCT.md` §21).

### Why not retry and rely on the stable `Message-ID`

The tempting version of this: keep the automatic retry, and let the reused
`Message-ID` mean receivers throw the duplicate away. Rejected, for three
reasons.

Receiver dedup is **unreliable and unobservable**. Several large providers
dedup on `Message-ID` within a window; a great many MTAs and self-hosted
servers do not, and the two copies may not even take the same path. Postio
cannot tell which kind it is talking to and would be gambling with the user's
correspondence on an answer it cannot look up.

The costs are **not symmetric**. A duplicate email is a social artifact
delivered to somebody else's inbox that the user cannot recall and did not
choose. A message that needs saying "send it again" is three seconds, taken by
a user who has been told exactly what happened. Automatic retry trades a cost
the user can absorb for one they cannot.

And it contradicts what the product already promises. `PRODUCT.md` §16 —
*"the user always knows what happened"* — and the reason `Drainer` fails
loudly rather than silently is stated in its own docs: an operation that
vanished silently is a message the user believes they filed and cannot find.
A send that silently doubled is the same failure wearing the opposite mask.

### Why not confirm before sending

A dialog before every send, or a "are you sure it didn't go?" prompt on
recovery, would interrupt thousands of ordinary sends to protect against a rare
one — the exact anti-pattern the command registry's `Recovery` policy exists to
prevent, and `Send` already carries `Recovery::Window` for the window that is
genuinely reversible. Postio has one modal dialog in the entire app and this is
not the second.

### Why not probe the Sent folder before every retry

Considered as a way to keep automatic retry safe: before resending, look for
the `Message-ID` in Sent, and only send if it is absent. Rejected because
absence is not evidence — a server that does not file sender copies makes the
probe always say "not there", so the retry proceeds and the duplicate happens
anyway, now with a network round trip and a false sense of safety in front of
it. The same probe is genuinely useful *after* the fact, where a positive
result is conclusive and a negative one only leaves the existing question
standing, which is where Decision 3 puts it.

## What the user sees

`DraftState` has `Editing`, `Queued`, `Sending`, `Sent`, `Failed` and
`Unconfirmed`, and every one is set by production code. Each is reachable
from a list and from the open message; none of them is a toast alone,
because a toast is not a place a message can be found again ten minutes
later.

What is on its way is in the **Outbox** (a view over the Drafts folder,
`docs/archive/specs/003-outbox-and-reserved-mailboxes`); Drafts holds what you are
writing, what did not go and what cannot be confirmed — a message you have
just sent does not sit in the folder that means *unfinished* (#1491). A row
says its state in one word (`postio_ui::row::send_state_word`), and in Focus
the open message's action row offers the verbs that settle it
(`postio_ui::focus_dialog::send_verbs`).

| State | Where | Row word | What the user can do |
|---|---|---|---|
| `Queued` | **Outbox** | "Waiting to send" | Cancel send (`mod+shift+x`), or undo (`mod+z`) within the undo-send window; Edit takes it off the queue |
| `Sending` | **Outbox** | "Sending" | Nothing. A cancel is refused, and says why, and a retry would risk a second copy. |
| `Sent` | Sent | — | The ordinary "Sent" toast. The draft row is gone; the message is in Sent. |
| `Failed` | Drafts | "Not sent" | Retry send (`mod+shift+y`); Edit; Discard. The draft carries the server's own reason, named: "The server rejected grace@example.net — 550 mailbox unavailable." Never "something went wrong". |
| `Unconfirmed` | Drafts | "Not confirmed" | Retry send, saying plainly that it may arrive twice; **Mark as sent** (`mod+shift+m`); Edit; Discard. "The connection dropped while this was being sent. It may have arrived. Checking your Sent folder." |

`Failed` may say "nothing was delivered" and mean it, because every failure
that reaches it — auth, sender or recipient rejection, message rejection,
configuration, and retries exhausted before the payload went — is on the safe
side of the boundary Decision 3 draws. That is the concrete user-facing payoff
of splitting the predicate: without it, `Failed` would have to hedge on every
send.

**Mark as sent** is a registry command with its own key (`mod+shift+m`) and
no undo: it settles a claim about the world rather than changing it, so the
correction for a wrong answer is to send again, a real act. It exists
because the user who checks with the recipient and learns
it did arrive otherwise has only two exits — discard, which throws the message
away, or send again, which duplicates it. An `Unconfirmed` draft with no honest
way out is a dead end, and nothing in Postio is a dead end.

Editing an `Unconfirmed` draft returns it to `Editing` and clears the reserved
`Message-ID`, per Decision 1. Editing a `Queued` draft takes it off the queue
first (#433). `Sending` is never editable: it is the one state where an edit
would change bytes already on the wire.

## Vocabulary

**"Unconfirmed"**, not "uncertain", "unknown" or "maybe sent". It names what is
missing — a confirmation — rather than describing a mood, and it is the word
that stays true when the confirmation arrives and the state resolves itself.
It belongs beside the words `/ux-architect` §2 fixes; a second spelling of
it anywhere is a bug in the design, not the code.

## Consequences

- **`postio-smtp` knows where its payload stopped.** `data` tracks whether
  the payload had begun being written when the transport failed, which is
  what `submission_is_indeterminate` reads.
- **The schema:** `drafts.rfc_message_id`, and `drafts.state`'s `CHECK`
  includes `'unconfirmed'`.
- **A fifth drain outcome**, which every `match` over `Outcome` answers, and
  a field for it on `DrainReport` and `DrainSummary`.
- **Two builds of the same queued draft get the same `Message-ID`**, and an
  edit changes it (`a_reserved_message_id_survives_every_rebuild`).
- **Tested with a transport that dies mid-`DATA`.** `postio-smtp`'s
  `ScriptedConnector::vanishing_after_the_payload`, and the sync suite's
  `an_unconfirmed_send_resolves_when_its_message_turns_up`.

## What would falsify this

- **`Unconfirmed` turning out to be common rather than rare.** The whole design
  assumes an interrupted `DATA` is unusual and worth a person's attention. If
  real use produces it weekly — a flaky mobile link, an aggressive
  middlebox — then a question the user is asked every week is a nag, and the
  right answer moves back toward automatic retry with the stable `Message-ID`
  doing the work. Worth counting before assuming.
- **Sent-folder confirmation resolving nearly everything.** If in practice
  almost every submission server files the sender's copy, the visible state is
  a transient the user rarely sees, and the elaborate copy above is more design
  than the case deserves.
- **A transport-level idempotency key appearing.** If a submission mechanism
  Postio speaks ever offers one — a future extension, or a non-SMTP submission
  path behind the same seam — the middle window closes properly and Decision 3
  becomes unnecessary rather than merely conservative.
