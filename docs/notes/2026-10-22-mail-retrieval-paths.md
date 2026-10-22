# Mail retrieval paths: where a body fails to arrive (2026-10-22)

An audit, prompted by filtered messages showing "Downloading this message" for ever (#1978, which is being fixed separately). Read-only except for the red tests named below, which live in the `audit-retrieval` worktree and are not committed. Everything here was read at `b4a96937d`.

## The path

1. **Discovery.** `postio-sync` `initial` (first enumeration), `resync` (CONDSTORE/QRESYNC `CHANGEDSINCE`, `UIDNEXT` arrivals, count-checked vanish detection, `UIDVALIDITY` change wipes the mailbox), `watch` (IDLE or `STATUS` poll). Headers only; `into_message` makes a `HeadersOnly` row.
2. **Header storage.** `MessageRepository::upsert_batch`: matches by `remote_id`, then `(validity, uid)`, then (rows with no identity) `Message-ID` in the same mailbox. A local move nulls `uid`, `remote_id`, `mod_seq` until the destination is synced.
3. **Background backfill.** `Backfill` (in memory; `body_state` is the durable record). Lanes: interactive, fresh (arrivals), background. `seed`/`top_up_backfill` read `needing_backfill_from`, skipping `body_failures` rows whose `mod_seq` still matches. `pump_body` takes `next_batch` (#1941), prefetches the text sections of the batch in one command, then runs two fetches at a time. `Outcome::Failed` and `Gone` go into `set_aside` for the life of the engine.
4. **On-open fetch.** Frontend `readings` -> host `opened` -> `AskedBodies::to_ask` (once per message per 60 s) -> `Host::fetch_body` -> `engine_for_message` -> `Engine::request_body` -> `backfill::request_body` (`backfill_candidate`: needs `uid`, `remote_id`, `body_state <> 'full'`, not `deleted_locally`) -> `request_now`.
5. **Parts.** `parts::part_bytes` (`request_payloads`, then poll up to 30 s), `raw_source` (`request_whole_message`, then poll), `fetch_carried` for forwards, `export::write_messages`. All route by the message's account.
6. **Frontends.** Mac: `postio-ffi` (`focus_message.rs` passes the real offline flag; `Engine.swift` re-reads the open message on `bodyLoaded`/`bodyFailed` only). GTK and TUI: `postio-client` `readings(.., false)`, re-read on the same two events.
7. **Reader.** `postio_session::reading::load_with_row`: `Failed` only when `body_failures` matches; otherwise `Offline` (caller's flag) or `Partial` whenever `body_state` has no body; `Missing`/`Empty` for a gone row or no text.

## Findings, ranked

| # | Finding | Where | Evidence | Issue |
|---|---|---|---|---|
| 1 | A header refresh of a known IMAP row resets `body_state`, `raw_blob_id`, preview, part map and attachment blobs: reading, flagging elsewhere or archiving a message brings back "Downloading" | `messages.rs` `write_update` via `upsert_batch` (only content-identity rows get `reuse_content_projection`) | three red tests (other client flags; own mark-read drains and next pass; archive then destination sync) | #1982 (p1) |
| 2 | A moved message with no `Message-ID` is inserted twice at the destination; the row in view keeps `uid IS NULL` for good | `find_awaiting_identity` is `Message-ID` only; the drainer discards the COPYUID mapping | red test: 2 rows, expected 1 | #1983 (p2) |
| 3 | View source, export and drag-out: "nothing to fetch" for any downloaded attachment-free message | `request_whole` -> `backfill_candidate` `body_state <> 'full'`; `raw_source` | red test | #1984 (p2) |
| 4 | A transient body failure reads as Downloading, `AskedBodies` suppresses a re-ask for 60 s, nothing re-reads; background failures are set aside until the app restarts (the "retry on reconnection" comments describe code that does not exist); `UidValidityChanged` on a fetch queues no resync | `fetch_body` marks only refusals; `settle_bodies`; `Backfill::finished`; `handle_link_transition` | red test for the reader; set-aside by reading | #1985 (p2) |
| 5 | Link blocked or waiting: GTK/TUI pass `offline=false` and promise a download that cannot run; Mac `Offline` plate is never re-read or re-asked when the network returns; queued requests are in memory | `pump_body` guards, `focus_message.rs`, `Engine.swift`, `open.rs` | reading | #1986 (p3) |
| 6 | A failed payload fetch waits 30 s, then says "still downloading" | `parts::wait_for_part` | reading | #1987 (p3) |
| 7 | Silent exits in `Host::fetch_body` (no engine) and `Backfill::request_body` returning `false` for a moved row with no UID | `lib.rs` `fetch_body`, `messages.rs` `backfill_candidate` | reading | #1978 (the other agent's; commented) |
| 8 | An empty or short section answer is stored as an empty body (`Body::Empty` for good); no check against the part's size | `fetch_text_parts` | reading, unverified | not filed |

Checked and fine: per-account routing for bodies, parts, forwards and exports (`engine_for_message`, #1965/#1969); Gmail and JMAP `fetch_part` serve `Whole` only and the backfill takes the whole-source path for them, and their ids survive a move; a `UIDVALIDITY` change wipes the mailbox so queued claims find `Gone`; a cancelled or short literal is `Disconnected`, not stored; excluded folders still fetch on open; thread, digest and search readings all go through `opened`.

## Overlap with #1978

Rows 2 and 7 are the same family: a moved row has no server identity until the destination is synced and re-adopted. Row 1 is a different cause that produces the same plate.
