# ADR 0016 — Full-mailbox backfill by default, folders optionally excluded

- **Status:** Accepted (2026-08-25), built; decided by the maintainer in response to #318
- **Decision:** every selectable folder backfills every message body, eventually, in the background, throttled by `BackfillPolicy`. There is no horizon. A folder is excluded only when the user says so.

## Rules
- Backfill tops up in batches until the folder is done. It is a background-lane, throttled, resumable process.
- Pacing is `BackfillPolicy` (`postio-sync::backfill`): `max_body_bytes` (5 MB), `pause_on_metered`, `pause_when_active`, `background` (`[sync] body_fetch`). Throttles change pacing, never the target. A starved interactive fetch is a throttling bug, not a reason for a horizon.
- Backfill is a disk-and-index axis. The list stays windowed over the paged store; a mailbox is never loaded into memory (CLAUDE.md, What the code must do).
- Opt-out is `mailboxes.backfill_excluded`, per folder, explicit and reversible, set in Settings (Sync & storage, "Back up locally"). `backfill::seed` queues nothing for an excluded folder; on-open fetches (`request_body`, `request_whole`) still work, and nothing already pulled is touched.
- Default is on. Onboarding asks nothing about it.

## Why
- "Fully usable offline after the first sync" (PRODUCT §15) fails for a body that was never pulled: it is a network round trip on open.
- Search is a primary way to navigate (PRODUCT §7). The body index covers only what is local, so a cap silently ends search coverage of older history.

## Revisit when
- Backfilling a very large real account starves interactive fetches or sync housekeeping: adjust `BackfillPolicy`.
