-- The thread list's index (ADR 0015, #306).
--
-- A folder shows one row per thread, and each row needs three things about
-- the part of that thread which is filed *in this folder*: its newest
-- message (the representative the row is drawn from), how many of its
-- messages are unread here, and whether any of them is flagged.
--
-- `idx_messages_thread (thread_id, received_at, id)` already existed and
-- answers "this thread's messages" — but not "this thread's messages in this
-- mailbox", so every one of those three questions would have walked the whole
-- conversation and filtered. Putting `mailbox_id` second makes each of them a
-- seek, and the descending tail means the representative is the first row the
-- seek lands on rather than the last.
--
-- Not a replacement for `idx_messages_thread`: threading itself walks a whole
-- thread across folders (#44), which is exactly what that index is for.
CREATE INDEX idx_messages_thread_mailbox
    ON messages (thread_id, mailbox_id, received_at DESC, id DESC);
