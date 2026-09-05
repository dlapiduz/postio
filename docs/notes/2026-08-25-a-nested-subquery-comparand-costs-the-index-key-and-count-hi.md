# A nested subquery comparand costs the index key — and `count(*)` hides it (#746)

Every search with hits took seconds on a real store (4.5 s at 320 matches,
~15 s at a full candidate pool) while the same searches ran in tens of
milliseconds on every bench corpus. A per-statement profile (rusqlite's
`trace` feature) put all of it in one statement: hydrate. Three lessons,
each of which cost this diagnosis a detour:

1. **SQLite cannot use a subquery as an index key.** Hydrate's
   `sender_times_seen` compared `contacts.address_normalized` against a
   *nested correlated subquery*. The plan shows what that costs:
   `SEARCH c USING INDEX idx_contacts_account_address (account_id=?)` — the
   address column silently drops out of the probe, so every hydrated
   candidate walked all 18k of the account's contacts, re-evaluating the
   inner recipients lookup per contact row. `O(candidates × contacts)`,
   ~14 ms per candidate. Hoisting the address into the row source and
   comparing against the plain column restored
   `(account_id=? AND address_normalized=?)`: the same workload dropped
   from 14.9 s to 6 ms. When a correlated subquery is slow, read its
   `EXPLAIN QUERY PLAN` line for the *columns in the probe*, not just the
   index name — the index being mentioned is not the index being used.

2. **A `count(*)` wrapper un-measures scalar subqueries.** The first replay
   wrapped the suspect statement in `SELECT count(*) FROM (...)` and
   "proved" it ran in 4 ms — SQLite prunes subquery columns nothing reads,
   so the expensive expressions never executed. Half a diagnosis chased a
   phantom because of it. To time a statement, run the statement: step
   every row and read every column.

3. **A cost that multiplies by a table's size is invisible while the bench
   leaves that table empty.** No bench corpus seeded `contacts`, so the
   scan multiplied by zero and `search_budget` stayed green through the
   whole regression. The corpus now carries 20k contacts. When a query's
   cost has the shape `rows × other_table`, the bench must populate
   *both* factors at real-mailbox scale.

The fix is pinned twice: a plan-shape test
(`hydrate_probes_contacts_by_address_key`) asserts the probe keys on the
address — deterministic, unlike a timing assertion — and the seeded bench
would blow its 100 ms budget by two orders of magnitude if the scan came
back.
