# ANALYZE makes the hot plans worse (#1708)

Question: would one `ANALYZE` when a mailbox's first sync completes give the
planner what it needs? #1707's lookup had picked `idx_messages_mailbox_remote_id`
over the Message-ID index because there were no statistics.

Measured on turso 0.8.x, on a store after the realistic first sync in
`crates/postio-sync/tests/sync_suite/scan_audit.rs` (3 mailboxes, 800
messages, threads, recipients, bodies): every statement the sync issued was
planned before and after `ANALYZE`, the second time on a fresh connection
(a connection reads the statistics when it opens).

**One plan improved, and it was already fine.** The identity fallback moved
from `idx_messages_mailbox_remote_id (mailbox_id=? AND remote_id=?)` to
`idx_messages_rfc_message_id`. Both are seeks.

**Five got worse**, including per-message statements:

| statement | before | after |
|---|---|---|
| recipients of a message (`recipients r JOIN addresses a`) | seek `a` by rowid | `SCAN addresses` |
| thread by `(account_id, subject)` | covering index seek | `SCAN threads` |
| backfill's `body_encoding_problems = 1` arm | `idx_messages_body_problems` seek | `idx_messages_list` walk of the folder |
| `accounts WHERE id = ?` | rowid seek | `SCAN accounts` |
| orphan rethread | `idx_messages_mod_seq` | `idx_messages_uid_read` |

The planner reads an index's average rows per key and drops a seek on a
column that is nearly constant (`body_encoding_problems` is 0 for almost
every row, so "= 1" looks like all of them) and, on a join, will scan the
small side. Statistics here are a way to lose a plan that was right.

Cost, for the record: `ANALYZE` over 10,000 messages took about 65 ms and
~20 ms over the audit store, and is a scan of every table and index. Not
free on a large encrypted store either.

**Decision: do not run ANALYZE.** Postio never has, nothing in the engine
creates `sqlite_stat1`, and `scan_audit.rs` asserts the store has none, so
that the audit keeps reading the plans production gets. Fix a plan with an
index the planner cannot misjudge, not with statistics. If this is revisited,
re-run the audit with `ANALYZE` and compare every statement, not the one you
meant to improve.
