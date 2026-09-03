# ADR 0026 — A saga's remove phase carries its own coordinates, and the confirmed identity goes on the row

- **Status:** Accepted (2026-09-03)
- **Date:** 2026-09-03
- **Decision by:** a `/ux-architect` session, on the question
  [#531](https://github.com/dlapiduz/postio/issues/531) was relabelled for:
  how does an inverse saga name the copy it must delete?
- **Issue:** [#531](https://github.com/dlapiduz/postio/issues/531)
- **Related:** [ADR 0005](0005-multiple-accounts.md) Q9 (the three-phase saga)
  and Q10 (`Attention`, and a view that cannot include an account says so),
  [ADR 0018](0018-jmap-and-gmail-backends.md) Q2 (`RemoteId`),
  [#188](https://github.com/dlapiduz/postio/issues/188) (the forward saga),
  [#289](https://github.com/dlapiduz/postio/issues/289) (why a queue row
  snapshots the identity it will need), `PRODUCT.md` §16 (undo)
- **Decision:** **an operation names the message it will act on by the identity
  its own queue row snapshotted, never by re-reading a message row at drain
  time** — which every other move and delete in the product already does, and
  which `cross_account::remove` alone does not. Alongside it, **phase 2 writes
  the confirmed identity onto the provisional target row**, and the provisional
  row stops being created with another server's identity on it.

---

## The premise both proposed options rest on, and why it is false

#531's own analysis offers two ways out and picks neither, because both were
believed to be unsafe in the mail-losing direction:

> `Message.server` is `(uid, uid_validity, remote_id)` and the append only
> proves the UID — writing a `remote_id` whose `uid_validity` is unknown puts a
> half-identified row in front of the sync engine's reconciliation.

**The append proves both.** RFC 4315's `APPENDUID` carries the destination
mailbox's `UIDVALIDITY` *and* the assigned UID, and the adapter uses both:

```rust
Ok(uid.map(|(uid_validity, uid)| UidMapping {
    source: Uid::new(uid),
    destination: Uid::new(uid),
    uid_validity: UidValidity::new(uid_validity),
    destination_remote_id: identity::remote_id(UidValidity::new(uid_validity), Uid::new(uid)),
}))
```

— `crates/postio-account/src/imap/mutate.rs:233`. The no-UIDPLUS fallback is no
weaker: `find_by_message_id` returns `identity::remote_id(live, uid)`, where
`live` is the `UIDVALIDITY` `ensure_selected` observed on the mailbox it just
searched (`mutate.rs:288`). **A `confirmed_remote_id` always carries a
generation.** There is no half-identified row to be afraid of.

**And a stale identity cannot mis-target.** `identity::wire_uid` refuses any
`RemoteId` whose packed generation is not the mailbox's live `UIDVALIDITY`,
answering `BackendError::UidValidityChanged` — its own words: *"a name from a
generation the server has abandoned … the caller's recovery is identical:
resync the mailbox, never retry the uid."* The worst case of a stale
coordinate is a resync, not a wrong expunge. That is the property that makes
this whole area tractable, and it is worth knowing before designing anything
here.

So the choice is not between two unsafe options. It is a free choice, and the
tree already contains the answer.

## The mechanism already exists, and `remove` is the only operation not using it

`operation_queue` has carried a `source_remote_id` column since the initial
schema, with the reason on the column:

```sql
-- The server identity the target had when the operation was enqueued. The
-- local row may be gone or renumbered by the time this drains.
source_remote_id     TEXT,
```

`enqueue` fills it automatically for any message-targeted operation, reading
the row *before* the caller's local write nulls it — the ordering #289 exists
to enforce, and the reason `enqueue` comes before the move in every `Move` and
`Delete` path. `CrossAccountRemove` is enqueued against the source row before
`set_deleted_locally` runs (`crates/postio-session/src/actions.rs:796`), **so
its snapshot is already correct today.**

`cross_account::remove` simply does not read it. It re-derives instead:

```rust
let remote_id = saga
    .source_message
    .and_then(|message| MessageRepository::new(connection).get(message).ok())
    .flatten()
    .and_then(|message| message.server.remote_id);
let (Some(path), Some(remote_id)) = (path, remote_id) else {
    // The source copy is already gone — another client removed it, or a
    // resync did. The move is complete either way.
```

That `else` is where the silence comes from. "No `remote_id` on the row" is
being read as "the server copy is gone", and it is not: it is also what a row
that never had one looks like, which is exactly the provisional target row an
inverse saga would hand it. The saga transitions to `done`, nothing is deleted,
and Postio records a completed move that removed nothing.

**Decision: phase 3 reads the identity off its own queue row.** Forward and
inverse both work, because `enqueue` snapshots whatever the row it targets
says. Phase 3 becomes unconditional rather than gaining the "my coordinates
came from somewhere other than the source row" branch #531 feared, and it stops
guessing: the `else` branch is reachable only when no identity was ever
snapshotted, which is a genuinely different fact and one worth logging as such
rather than settling as `done`. Falling back to the message row keeps queue
rows written before this change working.

## The confirmed identity goes on the target row

The snapshot mechanism only helps the inverse saga if the row it targets — the
provisional copy in the target account — carries the right identity by the time
the inverse `CrossAccountRemove` is enqueued. So:

**Phase 2 writes `confirmed_remote_id` onto the target message row** as well as
onto the saga. ADR 0005 Q9's *"which is why the confirmed target UID is
recorded"* reads like this was always the intent; the value is currently
computed and kept only where phase 3 does not look.

It is safe for the reasons above, and it *improves* the forward path. The
upsert matches a fetched message to an existing row by
`find_by_remote_id(mailbox_id, remote_id)`. Today the provisional row has no
identity the target account's next sync can match, so the sync's own copy is a
second row for the same message. After this, the row it already has is the row
it finds.

**And the provisional copy stops claiming another server's identity.**
`relocate_rows` builds it as `row.clone()` and clears two of the four fields:

```rust
copy.server.uid = None;
copy.server.uid_validity = None;
```

`remote_id` and `mod_seq` survive, so the provisional row in account B is
stored carrying the identity account A's server minted for it, and
`is_known_to_server()` answers `true` for a message the target server has never
seen. That is a defect on `main` independent of undo, it is a precondition for
everything above, and it is the *first* thing to fix — an inverse saga built on
top of it would send account A's UID to account B's server, and only
`wire_uid`'s generation check would stand between that and expunging an
unrelated message.

## What undo does, per phase

The forward saga's phase decides, and the two ends were already settled:

| Phase | `u` does |
|---|---|
| `copying` | **Landed.** Bookkeeping only: withdraw both queue operations, delete the provisional copy, un-hide the source, saga to `aborted`. Nothing reached either server. |
| `unconfirmed` | **Refuses, out loud, changing nothing.** ADR 0005 Q9's answer to an unprovable copy is stop and ask; an inverse saga on an unproven copy guesses in the one place the design says never to. |
| `confirmed` | **Abort the forward saga first**, then run the inverse. Its pending `CrossAccountRemove` has not run, so the source copy is still on A's server — and if it were left queued it would delete the source *after* the inverse restored it. `Confirmed → Aborted` is already a legal transition. |
| `done` | The inverse saga in full: append back to A, confirm, remove from B. |

**The inverse saga is the same machine, not a second one.** A new
`cross_account_moves` row with source and target exchanged, seeded from the
forward saga's `confirmed_remote_id`, run by the same two drainers. Phase 1's
idempotency-by-Message-ID is what makes the `confirmed` case correct without a
special path: the original is still in A, the inverse copy finds it, confirms
without appending, and the inverse remove takes B's copy away.

**No new `CommandId` and no new `UndoKind`.** `SPECS.len() == CommandId::ALL.len()`
is asserted and every row in the binding table must be pressable, so an
internal, unbound command would put a verb nobody should press into the palette
and the cheat sheet. `Actions::undo` branches on the entry before replaying
inverses, which is what the landed cancel path already does.

## What the user sees

**One sentence for every successful undo, whatever the phase.** ADR 0005 Q9
already says the user sees a cross-account move as one action; they do not know
there are two servers and must not have to. Undo is local-first like everything
else — the row reappears in the source and leaves the target immediately, and
the saga reconciles behind it — so at `copying`, `confirmed` and `done` the
observable outcome is identical and the wording is too. A toast that explained
the phase would be explaining Postio's implementation to someone who asked for
their message back.

**Failure is what is user-facing**, and it has a surface already: an inverse
saga that cannot complete raises `Attention` on the named account (ADR 0005
Q10), which the UI already renders. No new surface, no new overlay.

**A partial undo of a bulk move proceeds and says what it could not do.** Twelve
messages moved across accounts are twelve sagas, and one of them sitting in
`unconfirmed` must not pin the other eleven. Undo cancels and inverts what it
can and names the count it could not, in the spirit of ADR 0005 Q10's rule that
a view which cannot include an account says so and stays usable. Nothing is
half-applied per message: the message it skipped is untouched, exactly where
the user last saw it.

---

## Alternatives

**Have `remove` read `confirmed_remote_id` from the saga row.** #531's option
2, and nearly right — the saga is the authority for the *target* coordinates.
Rejected only because it solves the narrower problem: phase 3 would grow a
notion of "my coordinates came from the saga rather than the row", while the
queue's snapshot answers the same question for every operation in the product
and needs no new concept. Take the general mechanism over the special case when
the general one is already there and already filled.

**Write the confirmed identity onto the target row and leave `remove` as it
is.** Sufficient for the inverse saga, and it leaves phase 3 re-reading a row
at drain time — the pattern #289 was filed about, on the one operation that can
lose mail. It also leaves the `else` branch silently settling as `done`, which
is how this issue's original symptom (a `u` that reported success and did
nothing) got past everyone once already.

**Refuse undo past `copying` for good.** What `main` does today, and honest.
Rejected because `PRODUCT.md` §16 and the reversibility policy both say a
destructive command carries a non-`None` recovery, and a cross-account move is
the most destructive command in the product. "Undo works except on the one
operation you would most want it on" is the wrong place to stop.

---

## Consequences

- `relocate_rows` clears `remote_id` and `mod_seq` on the provisional copy.
  A defect on `main`, fixed first and separately.
- `cross_account::confirm` writes the confirmed identity onto the target
  message row as well as the saga.
- `cross_account::remove` reads `source_remote_id` off its queue row, falling
  back to the message row for rows enqueued before this; the "already gone"
  branch logs rather than silently settling.
- `Actions::undo` gains the inverse-saga branch for `confirmed` and `done`,
  aborting the forward saga first in the `confirmed` case.
- No schema migration: `operation_queue.source_remote_id` and
  `cross_account_moves.confirmed_remote_id` both already exist.
- The registry is untouched — no new `CommandId`, no new `UndoKind`, no new
  binding, no new cheat-sheet row.
