-- Waking due snoozes walked every message the account had, every 5s (#1237).
--
-- `Engine`'s poll tick calls `wake_due_snoozes` on every fire, for the life
-- of the process, and the query behind it asks a question whose answer is
-- almost always "none":
--
--     SELECT DISTINCT mailbox_id FROM messages
--      WHERE account_id = ?1 AND snoozed_until IS NOT NULL AND snoozed_until <= ?2
--
-- Nothing indexed `snoozed_until` as a leading column. `0005` put it fourth
-- and fifth in the list indexes, where it can be *tested* but never *sought*,
-- so the planner's best option was a walk: `SCAN messages USING INDEX
-- idx_messages_mod_seq` on a store with statistics, or a seek to the account
-- and a walk of everything under it without them. Every row read, none
-- returned.
--
-- On a plain SQLite file that would be a waste worth fixing. On this one it
-- is worse: the store is SQLCipher, so every page read is an AES-CBC decrypt
-- and an HMAC-SHA512. A ring-buffer profile of the running application
-- (#1216) caught the sync thread with `sha512_block_data_order_avx2` at
-- 24.80% of samples, over `pcache1Fetch` and `sqlite3BtreeTableMoveto` --
-- thousands of hashes, five seconds apart, to conclude there was nothing to
-- do.
--
-- Partial, for the reason `0010` gives about attachments: `messages` holds
-- two disjoint populations here, and the snoozed one is a handful. An index
-- keyed on `snoozed_until` without the WHERE would store one entry per
-- message, almost every one of them NULL, sorted and maintained on every
-- insert for nobody.
--
-- `mailbox_id` is carried so the SELECT is covering -- it is the only column
-- the query returns, and fetching it from the table would put the row
-- lookups back that the index exists to remove.

CREATE INDEX IF NOT EXISTS idx_messages_snoozed_due
    ON messages (account_id, snoozed_until, mailbox_id)
    WHERE snoozed_until IS NOT NULL;
