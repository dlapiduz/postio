# What five other mail clients know (2026-10-19)

Last reviewed: 2026-10-19, against Postio `577d7932`. Source inspection
only: nothing was built or benchmarked, so every benefit below is a
hypothesis until a test or a measurement says otherwise. The 2026-10-05
[architecture inventory](2026-10-05-reference-codebase-architecture-inventory.md)
covers Letter's shell in more depth; this pass is about sync, reliability,
size and features.

| Client | Revision | Stack | Lines (src) | Sync engine |
|---|---|---|---|---|
| Aerion | `f9bd8eb2b692` | Go, Wails, Svelte | ~143k | own, go-imap v2 |
| Hylki | `cf398ae7e804` | Rust, relm4 | ~167k | own, async-imap 0.10 |
| Lookout | `7f36a22eea63` | Rust, GTK | ~155k | own, vendored async-imap |
| Letter | `aee572fdefe2` | Vala, GTK | ~40k | Evolution's Camel |
| Stamp | `75ad9f05528b` | C, GTK | ~28k | Evolution's Camel |
| Postio | `577d7932` | Rust + Swift | ~350k (+~200k tests) | own, io-imap |

## None of them is smaller by design

The three with their own engines carry god files Postio has avoided:
Hylki's `src/app.rs` is 29k lines, Letter's `window.vala` and Lookout's
`window.rs` 11k each. Letter and Stamp are small because Camel and
Evolution Data Server do sync, accounts, OAuth, contacts and calendar for
them, and because they have almost no tests. Postio cannot delegate its
engine and stay local-first, so its size has to come down from inside:
logic that each frontend re-implements, and repository boilerplate.

## Their recurring bugs are one family

Every changelog repeats the same three classes, which is the best evidence
here of where a mail client breaks:

- **A partial answer read as a complete one.** Letter's "list shrinks after
  sync", Aerion's ghost messages, and Lookout's Gmail BODYSTRUCTURE that
  aborted a whole `FETCH 1:*`. Letter's fix is the rule worth keeping: a
  local row is deleted only on a listing that is known to have finished.
- **One bad message wedging a folder.** Aerion now marks a message whose
  body will not parse (`body_failed`, unless the bytes came back short of
  `RFC822.SIZE`, which looks like truncation); Lookout skips a folder after
  two prefetch failures.
- **Servers that under-report change.** iCloud never pushes flag changes to
  an idling session (Hylki verifies watched folders every 60 s); iCloud,
  Yahoo and Exchange get CONDSTORE wrong, so Aerion fully reconciles flags
  on folders under 2000 messages every pass and sweeps larger ones every
  tenth, advancing `HIGHESTMODSEQ` only when the flag pass succeeded.

## What applies to Postio

Found while comparing, checked against the code:

- `postio-sync/src/resync.rs`'s vanish check, taken when the count
  disagrees, re-fetches the headers of every known UID with no
  `CHANGEDSINCE`. That fetch is outside the skip counter
  (`postio-account/src/imap/fetch.rs`, which brackets only
  `changed_since` fetches), so a message `io-imap` silently drops is
  deleted locally. It is also the whole folder's headers in one unchunked
  call. `existing_uids` (one `UID SEARCH`, skip-counted) answers the same
  question.
- `COMPRESS=DEFLATE` is in the capability enum and never negotiated.
- The no-IDLE poll interval and the IDLE re-arm each have two defaults that
  disagree: 30 s in `imap/pool.rs` against 300 s in `watch.rs`, and 10 min
  against 9 min.

Patterns worth adopting:

- Aerion pipelines body batches (fetch N while N-1 is written) and bounds
  them by bytes as well as count (512 KB / 50, halved for folders with more
  than 1000 bodies missing).
- Hylki wraps every connection operation in a deadline and tests the
  wrapper against in-memory servers that stall mid-literal, never answer,
  or ignore `DONE`. It also remembers per account what a server refused
  (previews, Gmail labels, ENVELOPE) and stops asking.
- Lookout keeps one outbox table for undo send, scheduled send and offline
  send, with doubling retries and a quiet failure until the budget is
  spent. Postio already has all three; compare, do not copy.

Features Postio does not have yet: a sender-authentication banner from
`Authentication-Results` plus a display-name/address mismatch warning
(Lookout, Hylki); reply templates (Hylki); deleting everything a list sent
on unsubscribe, and auto-emptying old Junk and Trash (Hylki); remote
content blocked for CSS, fonts and redirects as well as images, including
in replies and forwards (Letter).

Not worth copying: Aerion's optimistic actions with no durable queue (it
loses them on a crash), Hylki's missing `UIDVALIDITY` handling and lack of
full-text search, and Aerion's full UID-set diff on every pass.

## The TUI stays

The TUI is a full third frontend (about 37k lines) with its own composer,
settings, onboarding, digest and rules. The maintainer keeps it. What can
shrink is logic it shares with GTK and the Mac that each re-implements,
which belongs in `postio-ui`.
