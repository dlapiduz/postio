# A partial index the planner will not read

2026-09-12, `feature/turso-store`.

## The finding

Turso's planner **will not use a partial index for a read**, and **does
enforce a partial `UNIQUE` constraint**. Both halves are pinned in
`crates/postio-storage/tests/turso_capabilities.rs`:

- `the_planner_does_not_use_a_partial_index`
- `a_partial_unique_index_is_still_enforced`

SQLite's planner does both. So a predicate that was free on SQLCipher is now a
choice between two different things depending on why the index has one.

## Why it matters here

Postio had twenty-two partial indexes, and they were doing two unrelated jobs
under one spelling.

**Sixteen were there to make the index small.** `idx_recipients_draft` is the
worked example: `recipients` holds two disjoint populations, message
recipients and draft recipients, and on the reference store 378,819 rows had a
`draft_id` of `NULL` and none had one set. `... WHERE draft_id IS NOT NULL`
turned a 6 MB index — 3.9% of a 163 MB database — into an empty one. On Turso
that same predicate makes the index *unreadable*, so the query it existed for
scans the table instead. The predicate is worse than useless: it costs the
write path an index to maintain and gives the read path nothing.

Those sixteen predicates are gone. The indexes are whole, the planner uses
them, and Postio's store is bigger than it was. That is the trade, taken
deliberately: a scan of `recipients` on every draft load is a user-visible
stall and six megabytes is not.

**Six were constraints**, and a constraint's predicate is not an optimisation:
`idx_settings_global_key` is what makes one global setting per key *true*, and
`idx_identities_one_default` is what stops an account having two default
identities. Turso enforces these, so they keep their predicates and keep
meaning what they said. Where such an index was *also* read through —
`idx_messages_uid`, hit once per message during a sync — a whole companion
index was added beside it, so the constraint stays exact and the read stays
indexed.

## The rule this leaves

**A `WHERE` on an index in `schema.rs` must be a constraint, never a size
optimisation.** If you are reaching for one to keep an index small, you are
writing an index nothing will read. If you are reaching for one to make a
uniqueness rule conditional, it works and it is the right tool — and if that
same index is also on a read path, add the unpredicated companion rather than
hoping.

`docs/decisions/0014-*` carries the store's threat model; this is not a
security property, it is a planner one, and it belongs with the engine notes.

## What could take it back

Nothing in Postio. This is Turso's planner, pre-1.0 and moving: if a later
release reads through partial indexes, the sixteen predicates are worth
restoring and the capability test above is what will notice, because it
asserts the current behaviour rather than skipping on it.
