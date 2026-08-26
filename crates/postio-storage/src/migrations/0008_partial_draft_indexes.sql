-- Two indexes that were never partial, and index nothing on a real account.
--
-- `recipients` and `attachments` are polymorphic: a row belongs to a message
-- XOR a draft (`message_id`/`draft_id`), the way the storage-schema
-- conventions in docs/engineering-notes.md already say. The `message_id`
-- indexes were correctly written `WHERE message_id IS NOT NULL`; their
-- `draft_id` siblings were not, so on an account with no drafts in flight
-- they index a column that is NULL on every single row -- measured on the
-- 81,744-message reference store: idx_recipients_draft alone was 6 MB, 3.9%
-- of the whole database, indexing nothing.
--
-- SQLite has no ALTER INDEX; recreate both as partial.
DROP INDEX idx_recipients_draft;
CREATE INDEX idx_recipients_draft ON recipients (draft_id, kind, position)
    WHERE draft_id IS NOT NULL;

DROP INDEX idx_attachments_draft;
CREATE INDEX idx_attachments_draft ON attachments (draft_id, position)
    WHERE draft_id IS NOT NULL;
