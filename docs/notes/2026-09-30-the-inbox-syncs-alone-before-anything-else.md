# The INBOX syncs alone before anything else (2026-09-30, #1709)

**The constraint:** a sync wave never admits a non-INBOX mailbox while an
`MailboxRole::Inbox` pass is queued or running, nor while INBOX's newest page
of bodies (one `seed_batch`) is being fetched, and it claims no background
body for any folder during INBOX's header pass. Future work on the wave, the
backfill or the watcher has to keep that, because Postio Focus's whole screen
is the inbox ("with Focus it is critical that the inbox headers and messages
sync first", maintainer).

## Why ranking is not enough

`sync_priority` ranks which mailbox *starts* first. That is not INBOX
*finishing* first: `sync_wave` admits up to `sync_lanes` mailboxes at once, so
without the rule below INBOX's header batches interleave with the next
folders' on the one store writer and the one connection budget, and every
committed batch in any lane claims bodies for whichever folder has rows.

## The rule

1. `next_to_sync` (`postio-runtime/src/engine.rs`) takes a queued INBOX ahead
   of everything and admits nothing else while one is queued or in a lane.
2. `claim_bodies_for_the_wave` claims nothing while an INBOX pass is pending.
3. When INBOX's pass seeds bodies, `inbox_page_pending` holds the other
   folders until that page is down. It is a *page*: no top-up reseeds INBOX
   while the flag is set, so the hold ends after `seed_batch` bodies, not the
   mailbox. The hold is dropped whenever no body fetch is in flight, so a
   paused or metered backfill cannot starve the other folders.
4. Keyed on the role in `State::inbox_ids`, set by `queue_every_mailbox`.
   Never a folder name.

It lives in the admission step, so it covers the first sync, a reconnect after
an offline gap (the link transition re-queues every mailbox), an IDLE wake and
an interrupted INBOX pass put back on the queue. Interactive body requests and
queued writes are served beside the lanes as before and are not held.

## Pinned by

`the_inbox_syncs_its_headers_and_newest_bodies_before_any_other_mailbox` in
`crates/postio-runtime/tests/runtime_suite/sync_wave.rs`: an order on the
mock's fetch log, plus the inbox list query returning the full page while the
other folders are unfinished.
