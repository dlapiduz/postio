-- Every `Message-ID` a thread has ever mentioned, present or not.
--
-- Threading has to answer one question on every incoming message: which thread
-- already knows about the ids this message names? The `messages` table cannot
-- answer it cheaply. `rfc_message_id` is indexed, but `reference_ids` is a
-- space-separated list, and finding a thread that *references* an id would mean
-- scanning the mailbox — which is exactly the cost the incremental threading
-- pass exists to avoid.
--
-- So a thread claims its ids here. That covers the case the `messages` table
-- cannot: a reply arriving before its parent claims the parent's id, and the
-- parent joins the thread that was waiting for it when it finally turns up.
--
-- One row per id per account: an id belongs to exactly one thread, and a
-- message that links two threads merges them rather than leaving the id
-- ambiguous.
CREATE TABLE thread_links (
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- Normalized by `RfcMessageId`: trimmed and angle-bracketed, so a lookup
    -- matches without further munging. Compared case-insensitively, because
    -- the wild does not agree on case.
    rfc_message_id TEXT    NOT NULL,
    thread_id      INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    PRIMARY KEY (account_id, rfc_message_id)
) WITHOUT ROWID;

-- "Which thread claims this id?" — the lookup on every incoming message.
CREATE UNIQUE INDEX idx_thread_links_lookup
    ON thread_links (account_id, rfc_message_id COLLATE NOCASE);
-- "What does this thread claim?" — rewritten when two threads merge.
CREATE INDEX idx_thread_links_thread ON thread_links (thread_id);
