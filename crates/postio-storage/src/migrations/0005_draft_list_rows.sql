-- Give a draft a place in the Drafts folder.
--
-- The message list is a windowed query over `messages`, and until now a draft
-- was not in that table at all: it was a `drafts` row, which is the composer's
-- live buffer. #51 then stopped the *synced copy* of a draft becoming a second
-- message row, on the grounds that the composer owns a draft this client wrote
-- and a read-only snapshot of a buffer still being typed into is not worth
-- listing beside it. What that left behind was a Drafts folder listing other
-- clients' drafts and nothing else, and a sidebar badge — which reads the
-- mailbox's cached count of message rows — saying 0 while the composer held a
-- draft. #166.
--
-- So a draft's list presence is a `messages` row this store writes, kept in
-- step by `DraftRepository::save`. Not the synced copy folded back in: a draft
-- has no server copy until an append has round-tripped, and a folder that only
-- listed your draft after a network exchange is what docs/PRODUCT.md §18's
-- local-first rule exists to forbid. The row is written the moment the draft
-- is, offline and always.
--
-- The alternative was to make the list's row identity a sum type over
-- `MessageId | DraftId`. That reaches `ListCursor`, `MessageSummary`, the
-- selection model and every `CommandId` target — the hottest path in the
-- application — to solve a problem one nullable column solves.

ALTER TABLE drafts ADD COLUMN message_id INTEGER
    REFERENCES messages(id) ON DELETE SET NULL;

-- Partial, because most rows in most stores have one and the lookups that
-- matter are "which message row is this draft's" and "is this message row a
-- draft's".
CREATE INDEX idx_drafts_message ON drafts (message_id) WHERE message_id IS NOT NULL;
