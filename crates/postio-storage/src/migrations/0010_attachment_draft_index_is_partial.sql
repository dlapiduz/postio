-- `idx_attachments_draft` indexed every attachment, not every draft's (#381).
--
-- `attachments` holds two disjoint populations -- rows belonging to a stored
-- message and rows belonging to a draft -- and the table's own CHECK says it
-- is exactly one of the two. Drafts are a handful; messages are the mailbox.
-- So an index keyed on `draft_id` without a WHERE stores one entry per
-- message attachment, every one of them NULL, sorted and kept for nobody.
--
-- Measured on the reference store (81,744 messages, 163 MB): `recipients`
-- held 378,819 rows of which zero had a `draft_id`, and its equivalent index
-- was 6 MB -- 3.9% of the whole database -- before it was made partial.
-- `attachments` is the same shape and was the half that got missed: its
-- sibling `idx_recipients_draft` has carried `WHERE draft_id IS NOT NULL`
-- since the schema was consolidated in #610, and this one did not.
--
-- The key is unchanged, so every read that used it still does:
-- `DraftRepository::fill` asks `WHERE draft_id = ?1 ORDER BY position, id`,
-- and `draft_id = ?` is the proof the planner needs that the query cannot
-- want the rows the WHERE leaves out. `storage_suite/draft_indexes.rs`
-- asserts both halves, because a partial index the planner declines to use
-- still returns the right rows -- by scanning the table.

DROP INDEX IF EXISTS idx_attachments_draft;
CREATE INDEX idx_attachments_draft
    ON attachments (draft_id, position)
    WHERE draft_id IS NOT NULL;
