# ADR 0021 — Sending is at-most-once, and an interrupted send is reported rather than guessed

- **Status:** Accepted (2026-08-28)
- **Decision:** Postio guarantees at-most-once submission and never guesses. The `Message-ID` is minted once per send attempt series; the durable commit point is the instant SMTP accepts; an SMTP session that dies after the payload began is not retried but settles as `Uncertain` and becomes a visible `Unconfirmed` draft.

## Rules
- `drafts.rfc_message_id` is minted by `queue_send` / `queue_send_at` in the transaction that writes the `Operation::Send` row. `outgoing::build` takes it as a parameter, and every attempt carries the same id. It is cleared when the draft returns to `Editing`, because an edited draft is a different message. Receiver dedup is a side effect, never a promise.
- The draft goes to `Sending` (committed) immediately before `send_message`, after connect and auth. It goes to `Sent`, with the acceptance time, in one transaction immediately after `Ok`, before `quit()` or the Sent-folder `APPEND`.
- `send::resolve`: `Sent` is `Obsolete`, never rebuilt or resubmitted. `Sending` on a fresh process is `Uncertain`, never resent. Every ambiguity resolves toward asking, because a duplicate is irrecallable and a resend is three seconds.
- `SmtpError::submission_is_indeterminate()` is true for `Disconnected`, `TimedOut`, `Io` or `Cancelled` raised once the payload began; `postio-smtp` tracks the boundary in `data`. The drainer checks it before `is_transient`. Callers branch on predicates, never variants.
- `Outcome::Uncertain { reason }` settles the queue row (done, no further attempts) and surfaces separately in `DrainReport` and `DrainSummary`. It is neither a failure nor a success.
- Postio resolves an `Unconfirmed` draft itself: on the next sync of Sent, a message carrying the reserved `Message-ID` marks it `Sent` and files the copy. This is ordinary sync of a folder the user has.
- Undo-send is a cancel against the queue, refused once the draft leaves `Queued`.
- `Failed` means nothing was delivered: every failure that reaches it is before the payload.
- `Sending` is never editable. Editing `Queued` takes it off the queue first.
- States and where they show: `Queued` and `Sending` in the Outbox ("Waiting to send", "Sending"); `Failed` ("Not sent") and `Unconfirmed` ("Not confirmed") in Drafts; `Sent` in Sent. A failed row names the server's reason, never "something went wrong".

| State | Verbs |
|---|---|
| `Queued` | cancel send (`mod+shift+x`), undo within the window, edit |
| `Sending` | none; a cancel is refused and says why |
| `Failed` | retry send (`mod+shift+y`), edit, discard |
| `Unconfirmed` | retry (warns it may arrive twice), **Mark as sent** (`mod+shift+m`, no undo), edit, discard |

- The word is "Unconfirmed", not "uncertain" or "maybe sent". An `Unconfirmed` draft always has an honest exit.

## Why
- A retry that rebuilds the message mints a new `Message-ID`. A drop during `DATA` looks transient but is not. SMTP has no idempotency key, so the middle window cannot be closed, only decided.
- Rejected: auto-retry on a stable `Message-ID` (receiver dedup is unobservable), a confirm dialog before every send, and probing Sent before each retry (a server that files no copies always says "not there").

## Revisit when
- Native submission (JMAP `EmailSubmission`, Gmail send) offers an idempotency key (ADR 0018).
