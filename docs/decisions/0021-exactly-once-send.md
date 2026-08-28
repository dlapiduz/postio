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

## What the tree does today

Three facts, all of them load-bearing, and none of them obvious from reading
`send.rs`'s reassuring module docs alone.

**Every attempt is a different message.** `outgoing::assemble` calls
`generate_message_id` on every build (`crates/postio-model/src/outgoing.rs`),
and there is a test asserting exactly that —
`each_build_gets_a_fresh_message_id`. `Drainer::resolve` calls
`send::resolve`, and therefore `outgoing::build`, on *every* drain attempt. So
the second delivery of a retried send is not a duplicate any receiving system
can recognise. It is a second, distinct message that happens to say the same
thing. Whatever else this ADR decides, the current retry path is the worst
available shape: it can duplicate, and it has deliberately destroyed the one
piece of evidence that would let anyone downstream notice.

**A dropped connection during `DATA` retries.** `SmtpError::Disconnected` is
`is_transient() == true` (`crates/postio-smtp/src/error.rs`), so
`outcome_from_smtp_error` returns `Outcome::Retry`, so the drainer defers and
sends again — up to `RetryPolicy::max_attempts`, eight, spanning about twenty
minutes. `send_message` writes the payload and reads the server's reply to the
terminating `.` inside one `self.data(...)` call, so a connection that dies
between those two events is reported the same way as one that died before
`MAIL FROM`. Those two cases are not the same case.

**The crash window contains a network round trip.** `send.rs`'s own docs call
the crash window "vanishingly narrow". It is not. The durable fact that stops
a resend is the *deletion of the draft row*, because `send::resolve` treats a
missing draft as `Obsolete`. That deletion is the second-to-last line of
`file_sent_copy`, and between SMTP acceptance and it sit: `session.quit()`, an
IMAP `APPEND` of the whole message to the Sent mailbox, a blob write, a
`messages.create`, threading, and a body write. On a slow link the `APPEND`
alone is seconds. The window is not narrow, it is the largest single piece of
network work in the whole send path — and #423 means real messages now travel
through it.

Fourth, and it is the user-facing half of the same problem: **`DraftState`
has five variants and production code sets exactly two.** `Editing` and
`Queued` are written; `Sending`, `Sent` and `Failed` appear only in tests. A
send that fails permanently marks its queue row failed, emits one
`Event::Error` toast that scrolls away, and leaves the draft sitting in
`Queued` forever — a message the user believes they sent, that will never be
retried, in a state whose name says it is about to be.

## The three windows

| Window | Can the client know what happened? | Closable? |
|---|---|---|
| Before the payload is submitted — connect, auth, `MAIL FROM`, `RCPT TO` | Yes. The server has nothing. | Already closed. Retry is correct and safe. |
| Between submitting the payload and reading the final reply | **No. Not by any means SMTP offers.** | Not closable. Must be *decided*. |
| Between the final reply and the local record of it | Yes, if the record is written first. | Closable, and currently wide open. |

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
2. **Immediately after `send_message` returns `Ok`**, in one SQLite
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

This also fixes a live hazard in undo-send. `Operation::Send` has no inverse;
undoing a send is a *cancel against the queue* (`operation.rs`), and today
nothing stops that cancel landing on a row whose SMTP transaction is already
open — cancelling a message that is being delivered, and telling the user it
was recalled. With mark 1 in place, the cancel is refused the moment the draft
leaves `Queued`, and `Recovery::Undo` on `CommandId::Send` becomes an honest
claim about a window that actually has an end.

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

`Outcome` gains a fifth variant, `Uncertain { reason }`. This is an
architectural addition and worth naming as one: the drain vocabulary has been
four answers since it was written, `DrainReport::failed` means *did not
happen*, and the runtime turns it straight into `Event::Error`. An interrupted
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
prevent, and `Send` already carries `Recovery::Undo` for the window that is
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

`DraftState` gains `Unconfirmed`, and the four states that exist stop being
decorative. Every one of these is reachable from the Drafts list and from the
composer; none of them is a toast alone, because a toast is not a place a
message can be found again ten minutes later.

| State | Copy | What the user can do |
|---|---|---|
| `Queued` | "Sending when you're back online." — or nothing at all while a drain is due; a queued send that leaves within a second should not announce itself. | `u` cancels, within the undo-send window |
| `Sending` | "Sending…" | Nothing. The cancel is refused, and says why. |
| `Sent` | The ordinary "Sent" toast. The draft row is gone; the message is in Sent. | — |
| `Failed` | The server's own reason, named: "The server rejected grace@example.net — 550 mailbox unavailable." Never "something went wrong". | `Enter` opens it, editable again; `ctrl+Return` sends again; `d` discards |
| `Unconfirmed` | "Not confirmed — the connection dropped while this was being sent. It may have arrived. Checking your Sent folder." | `Enter` opens it; **Mark as sent**; `ctrl+Return` sends again, saying plainly that it may arrive twice; `d` discards |

`Failed` may say "nothing was delivered" and mean it, because every failure
that reaches it — auth, sender or recipient rejection, message rejection,
configuration, and retries exhausted before the payload went — is on the safe
side of the boundary Decision 3 draws. That is the concrete user-facing payoff
of splitting the predicate: without it, `Failed` would have to hedge on every
send.

**Mark as sent** is a new command in the registry, palette-only (ten commands
already carry no default binding), `Recovery::Undo`, available wherever a
draft is. It exists because the user who checks with the recipient and learns
it did arrive otherwise has only two exits — discard, which throws the message
away, or send again, which duplicates it. An `Unconfirmed` draft with no honest
way out is a dead end, and nothing in Postio is a dead end.

Editing an `Unconfirmed` draft returns it to `Editing` and clears the reserved
`Message-ID`, per Decision 1. That is a deliberate act with a consequence, and
it is the same act #433 is deciding for `Queued` drafts; whatever that issue
settles must not make `Sending` editable, which is the one state where an edit
would change bytes already on the wire.

## Vocabulary

**"Unconfirmed"**, not "uncertain", "unknown" or "maybe sent". It names what is
missing — a confirmation — rather than describing a mood, and it is the word
that stays true when the confirmation arrives and the state resolves itself.
It is a new word in the product's vocabulary, decided here rather than
silently, and it belongs beside the ones `/ux-architect` §2 already fixes; a
second spelling of it anywhere is a bug in the design, not the code.

## Consequences

- **`postio-smtp` learns where its payload stopped.** The new predicate needs
  `data` to know whether the payload had begun being written when the
  transport failed. This is the only change outside `postio-sync` /
  `postio-storage` / `postio-model`, and it is the one that everything else
  depends on.
- **A schema change**: `drafts.rfc_message_id`, and a `CHECK` widened for
  `'unconfirmed'`. The migrations test asserting the table list is unaffected.
- **A fifth drain outcome**, which every `match` over `Outcome` must answer,
  and a new field on `DrainReport` and `DrainSummary`.
- **`send.rs`'s module docs are wrong and must be rewritten.** "A known gap …
  vanishingly narrow" understates what is there by an IMAP round trip, and
  that sentence is why the gap survived being read.
- **`each_build_gets_a_fresh_message_id` inverts**: the invariant becomes that
  two builds of the same queued draft get the *same* id, and that an edit
  changes it.
- **The undo-send window gets a real end**, and `Recovery::Undo` on `Send`
  stops being a claim nothing enforces.
- **Testing this needs a backend that can die mid-`DATA`.** The `MailBackend`
  mock does not model a transport that accepts a payload and then vanishes, and
  the corpus does not help. The two acceptance tests #461 asks for both need
  that fixture; it is the largest piece of work this decision creates.

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
