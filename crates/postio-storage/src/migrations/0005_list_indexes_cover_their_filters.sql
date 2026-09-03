-- The list indexes carry the columns the list filters on (#638).
--
-- Every list query ends in the same two predicates — the message is not
-- locally deleted, and it is not still snoozed — on top of whatever the scope
-- names:
--
--     WHERE messages.mailbox_id = ?1
--       AND messages.deleted_locally = 0
--       AND (messages.snoozed_until IS NULL
--            OR messages.snoozed_until <= strftime('%s','now') * 1000)
--     ORDER BY messages.received_at DESC, messages.id DESC
--
-- The four indexes below supplied the scope column and the order, and neither
-- filter column. That is invisible on the first page and ruinous on a jump:
-- only rows that *pass* the WHERE count toward an OFFSET, so SQLite has to
-- evaluate both predicates for every row it skips, and evaluating them meant
-- fetching the table row. A jump halfway down a hundred thousand messages was
-- therefore fifty thousand table-row fetches to return fifty rows.
--
-- Under SQLCipher each of those fetches can be a page decrypt, which is why
-- this looked like a cipher problem: measured on the cold-jump bench, the
-- encrypted store took 207ms and a plaintext one 59ms, so the cipher was 71%
-- of it. It was 71% of work that should not have happened. With these columns
-- in the index the skip never leaves it, and only the fifty returned rows
-- touch the table: the same encrypted jump measures 3.5ms.
--
-- `cache_size` is not the lever here, despite `db.rs` naming it the first one
-- to reach for: raising it from 16 MiB to 256 MiB moved the encrypted case
-- from 207ms to 213ms, which is nothing. The same was true of #619.
--
-- The cost is index size and a little more work per write, on four indexes
-- over two narrow columns — an integer flag and a nullable timestamp. That is
-- the right trade for turning a user-visible action from a fifth of a second
-- into something imperceptible.
--
-- `id DESC` stays where it is in each key: it is the tiebreaker the cursor
-- pages on, and the filter columns go after it so the ordering the index
-- supplies is untouched. `idx_messages_thread` keeps its ascending order for
-- the same reason — a thread reads oldest first.

DROP INDEX IF EXISTS idx_messages_list;
CREATE INDEX idx_messages_list
    ON messages (mailbox_id, received_at DESC, id DESC, deleted_locally, snoozed_until);

DROP INDEX IF EXISTS idx_messages_account_list;
CREATE INDEX idx_messages_account_list
    ON messages (account_id, received_at DESC, id DESC, deleted_locally, snoozed_until);

DROP INDEX IF EXISTS idx_messages_recency;
CREATE INDEX idx_messages_recency
    ON messages (received_at DESC, id DESC, deleted_locally, snoozed_until);

DROP INDEX IF EXISTS idx_messages_thread;
CREATE INDEX idx_messages_thread
    ON messages (thread_id, received_at, id, deleted_locally, snoozed_until);
