# A pragma that writes must not be in the per-connection batch (2026-09-04, #381)

`auto_vacuum = INCREMENTAL` went into `db::PRAGMAS` — the batch `configure`
applies to every connection the pool opens — because that is where pragmas
live and it needs to be set before the schema exists. Both halves of that
sentence are true and the conclusion is wrong.

**Setting `auto_vacuum` is a write.** On a store that already has tables it
cannot take effect and SQLite says so by doing nothing — but it takes the
write lock to find out. In the per-connection batch that means *every*
connection checkout attempts a write, so a reader checked out while a write
transaction is open blocks on it until `busy_timeout` gives up.

That is the exact inverse of the property the batch exists to provide. From
`PRAGMAS`' own doc comment: *"`journal_mode = WAL` — readers do not block the
writer and the writer does not block readers. This is the pragma the whole
local-first design rests on: the UI reads while sync writes."*

`storage_suite`'s `a_read_proceeds_while_a_write_transaction_is_open` caught
it, deterministically, five times out of five, at 5.04s against a
`recv_timeout` of 5s — the `busy_timeout` to the millisecond. Worth noting
how it presented: as one failure in a 474-case suite run while the machine
was loaded, which is exactly the shape of the flake in #1015, and the
5-second timeout would have made "it timed out under load" a comfortable
place to stop. What separated them was running it alone: a flake stops
reproducing and this did not.

**The fix is where, not whether.** The pragma is set in `configure`, gated on
the probe it already runs: `SELECT count(*) FROM sqlite_schema` answering
zero means the database has no schema yet, which is both the only moment
SQLite will accept `auto_vacuum` and a moment with no writer to contend
with. It goes before `PRAGMAS`, because `journal_mode = WAL` writes the
header and SQLite will not move `auto_vacuum` afterwards.

**The general rule:** `PRAGMAS` is for settings that are per-connection and
read-only to apply. Anything that touches the database *header* —
`auto_vacuum`, `page_size`, `journal_mode` on an existing file — belongs on
the creation path or behind an explicit one-time call, never in a batch that
runs on every checkout.
