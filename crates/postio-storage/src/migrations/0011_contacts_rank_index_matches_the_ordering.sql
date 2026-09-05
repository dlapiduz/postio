-- `idx_contacts_rank` named an order nothing asks for (#990).
--
-- It was written for the ordering ADR 0007 Q6 originally described --
-- `(times_seen DESC, last_seen_at DESC)`, frequency first. #424 changed
-- autocomplete to lead with recency and #430 corrected the ADR to say so;
-- the index was part of neither, so since #424 it has indexed one order
-- while `ContactRepository::search` asked for another. Silent, because the
-- *results* stayed correct: SQLite simply sorted them itself.
--
-- Swapping its two columns is the obvious fix and does not work. The
-- ordering leads with a band --
--
--     CASE WHEN source = 'mail' THEN 1 ELSE 0 END, last_seen_at DESC,
--     times_seen DESC, id
--
-- so that a contact the user created outranks one harvested from a header,
-- and SQLite can only satisfy an ORDER BY from an index when the leading
-- terms match *exactly*. Measured on 20,000 contacts, an index of
-- `(last_seen_at DESC, times_seen DESC)` still plans as `SCAN contacts` plus
-- `USE TEMP B-TREE FOR ORDER BY`. An index that leads with the same
-- expression plans as `SCAN contacts USING INDEX idx_contacts_rank`, with no
-- sort, and does so without `ANALYZE`.
--
-- Hence an expression index, which is unusual here and is the only shape
-- that works. `id` is on the end because the ordering ends there: a
-- tie-break that the index does not carry is a tie-break SQLite has to sort
-- for, which is the whole cost again.
--
-- Why it is worth a migration rather than a note: autocomplete runs on every
-- keystroke of a recipient, and the popup opens on the *empty* prefix -- the
-- case where nothing narrows the candidates and every contact is one. A
-- temporary b-tree over the whole address book, per keystroke, is what the
-- 16 ms interaction budget cannot hold, and `LIMIT 20` does not help: the
-- sort has to see every row before it knows which twenty come first.
--
-- `storage_suite/contact_rank_index.rs` asserts the plan, the shape and the
-- order the rows come back in -- all three, because an index the planner
-- declines to use still returns the right rows by scanning, and an index it
-- does use can return the wrong ones.

DROP INDEX IF EXISTS idx_contacts_rank;
CREATE INDEX idx_contacts_rank ON contacts (
    (CASE WHEN source = 'mail' THEN 1 ELSE 0 END),
    last_seen_at DESC,
    times_seen DESC,
    id
);
