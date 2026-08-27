# ADR 0018 — JMAP and Gmail REST backends, on Pimalaya crates

- **Status:** Accepted — maintainer-directed (2026-08-27)
- **Date:** 2026-08-27
- **Issue:** [#540](https://github.com/dlapiduz/postio/issues/540), under
  [#2](https://github.com/dlapiduz/postio/issues/2)
- **Related:** ADR 0001 (the `MailBackend` seam), ADR 0005 (one engine per
  account), ADR 0006 (OAuth; the token both new backends authenticate
  with), [#195](https://github.com/dlapiduz/postio/issues/195) (CASA),
  CLAUDE.md "Pimalaya first" (#537)
- **Decision:** Postio grows two more `MailBackend` implementations —
  **JMAP via `io-jmap`** and **Gmail REST via `io-gmail`** — selected per
  account by a `backend` preference list on the preset row. The enabling
  change is a **backend-neutral remote identity** on the message row;
  UIDVALIDITY generations become the IMAP adapter's private concern. The
  first shipping slice is the JMAP backend against Fastmail; Gmail REST
  follows once credentials exist for it (own-client first, verified
  client after #195).

---

## Why now

The maintainer's direction is future supportability on the Pimalaya
stack. Concretely: Fastmail's native protocol is JMAP — richer sync
(`/changes` beats CONDSTORE emulation), server-side threading, push
without IDLE, and submission without SMTP. Gmail's REST API is the
Gmail-native story — labels as labels, history-based delta sync — and
Google's long arc bends away from raw IMAP for third parties. Both
`io-jmap` and `io-gmail` reached 0.3.0 (2026-08-15) with the full mail
surface, in the same I/O-free coroutine style as the five Pimalaya crates
Postio already stands on.

What does **not** change: IMAP+SMTP remains the default and the
reference; a provider with no preset row keeps working exactly as today.
Providers are data — the backend choice is one more field of the row,
never a branch on a vendor name.

## Q1 — Where do the backends live?

Two new crates, `postio-jmap` and `postio-gmail`, each depending on its
Pimalaya wire crate and on `postio-imap` **with
`default-features = false`** — the crate that (wart acknowledged in ADR
0006 Q2) owns the `MailBackend` trait, the keyring, and OAuth. They
implement the trait; nothing above the composition root learns a new
name. The engine keeps one code path.

`check-crate-boundaries.py` grows both: no GTK, no SQL, no `io-imap`
types — the same leaf discipline as the crates they sit beside.

## Q2 — Remote identity, or: what `Uid` was hiding

`MailBackend` speaks `Uid` (u32) + `UidValidity` because IMAP does. JMAP
`Email` ids and Gmail message ids are opaque strings, immutable and
server-wide — they have no generations to invalidate and do not fit in a
u32.

**Decision: `messages.remote_id TEXT` joins the row as the
backend-neutral identity; `uid`/`uid_validity` stay as the IMAP
adapter's own columns.** The trait's message-addressing surface moves to
an opaque `RemoteId` newtype; the IMAP adapter derives its wire
`Uid` from it and keeps the generation dance — resync-on-UIDVALIDITY —
entirely behind the seam, where ADR 0001 always said protocol details
belong. The sync engine stops knowing what a generation is; "this
mailbox needs a full resync" was already a seam answer
(`needs_resync`), and stays one.

This is the enabling slice and it is deliberately first: every other
slice reads or writes identity, and retrofitting it later means
migrating live stores twice.

## Q3 — Delta sync

The engine's pull machinery asks IMAP-shaped questions. JMAP answers a
different one — `Email/changes` + `Email/queryChanges` against a state
string; Gmail answers with `history.list` against a `historyId`.

**Decision: emulate first, native seam later.** The adapters store their
state cursor where the sync state row already keeps its high-water mark,
and translate the engine's "what changed since" into their native delta
call. That ships JMAP sync without touching `postio-sync`. A native
`SyncStrategy` seam — letting a backend drive its own delta shape, push
included (`io-jmap` ships the RFC 8620 EventSource) — is a later slice,
taken when the emulation's cost is measured rather than guessed.

## Q4 — Gmail labels

A Gmail message has labels, not a folder. Postio already has
`message_labels`; the adapter maps system labels (INBOX, SENT, TRASH…)
onto the mailbox roles and everything else onto labels. The one rule
worth stating now: **archive means remove-INBOX-label**, which is what
the archive verb already means to a Gmail user, and the adapter owes the
engine the same "one operation per verb" contract IMAP gives it.

## Q5 — Authentication and the scope wall

Both backends authenticate with the OAuth tokens ADR 0006 built —
`TokenSource` is already the seam, and #533/#534 carry the mechanism to
the sessions and the sign-in to the wizard. Gmail's REST mail scopes are
**restricted scope**: usable today with the user's own client
credentials or a broker (the Q1 strategies), and with Postio's verified
client only after #195 clears CASA. The Gmail preset row therefore
advertises `backend = ["imap", "gmail"]` until #195, flipping preference
when the verified client exists — a data change, not a release.

## Q6 — Sending

JMAP has `EmailSubmission`; Gmail REST has `users.messages.send`. First
slices keep SMTP for both (Fastmail and Gmail both speak it, and the
send pipeline — drafts, BCC discipline, sent-copy filing — is tested
against it). Native submission is its own later slice per backend, taken
for the providers where SMTP is the worse path, not on principle.

## Alternatives

**Keep IMAP only.** Zero cost now; leaves Fastmail on an emulation of
what its server natively offers, and leaves Gmail hostage to IMAP's
future at Google. Rejected by direction.

**A separate abstraction above `MailBackend`.** A "transport" layer per
protocol family. Rejected: the trait *is* the abstraction, ADR 0001's
whole point; a second layer would mean every verb crossing two seams.

**Wait for the crates to reach 1.0.** The 0.x cadence is the reason the
"Pimalaya first" rule exists — waiting is how #537's gap happened. The
adapters pin minor versions and the mock-driven tests are the
compatibility net.

## Consequences

- Two new crates; two new rows in `check-crate-boundaries.py`.
- One migration (remote identity) that every existing store crosses
  once, IMAP adapters filling `remote_id` from `uidvalidity:uid`.
- The preset schema gains `backend` (a preference list, default
  `["imap"]`); the Fastmail row advertises jmap once the backend ships.
- The slices are tracked under the initiative epic this ADR lands with;
  work follows the initiative-branch flow (`--base feature/backends`).
