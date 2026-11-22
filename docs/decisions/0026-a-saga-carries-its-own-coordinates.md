# ADR 0026 — A saga's remove phase carries its own coordinates, and the confirmed identity goes on the row

- **Status:** Accepted (2026-09-03)
- **Decision:** An operation names its target by the identity its own queue row snapshotted (`operation_queue.source_remote_id`), never by re-reading a message row at drain time. Phase 2 of a cross-account move writes the confirmed identity onto the provisional target row.

## Rules
- `cross_account::remove` reads `source_remote_id` off its queue row (message row only as fallback). The "already gone" branch fires only when no identity was ever snapshotted, and it logs.
- `cross_account::confirm` writes `confirmed_remote_id` onto the target message row as well as the saga.
- The provisional copy clears `uid`, `uid_validity`, `remote_id` and `mod_seq`, so `is_known_to_server()` is false until confirmed.
- Undo of a cross-account move by saga phase:
  - `copying`: withdraw both queue operations, delete the provisional copy, unhide the source, saga `aborted`.
  - `unconfirmed`: refuse out loud, change nothing (ADR 0005 Q9).
  - `confirmed`: abort the forward saga first, then run the inverse.
  - `done`: run the inverse saga in full.
- The inverse saga is the same machine: a new `cross_account_moves` row with source and target swapped, seeded from `confirmed_remote_id`, run by the same drainers. Phase 1 idempotency by Message-ID makes the `confirmed` case correct.
- No new `CommandId` and no new `UndoKind`. `Actions::undo` branches on the entry.
- Every successful undo says one sentence, whatever the phase. A failed inverse raises `Attention` on the named account (ADR 0005 Q10).
- A bulk undo proceeds past messages it cannot undo and names the count skipped. A skipped message is untouched.

## Why
- APPENDUID carries UIDVALIDITY with the UID, so a confirmed id always has a generation. `identity::wire_uid` refuses a stale generation with `UidValidityChanged`. The worst case is a resync, never a wrong expunge.
- The queue snapshot is the general mechanism (#289). Reading the saga's coordinates in phase 3 would add a special case, and re-reading a row at drain time is what lost mail.
- Refusing undo past `copying` leaves the most destructive command without recovery (`PRODUCT.md` §16).

## Revisit when
- A backend cannot prove its identity's generation.
