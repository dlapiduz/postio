-- The server coordinates a Move or Delete needs, snapshotted at enqueue.
--
-- The local half of a move nulls the message row's uid/uid_validity in the
-- same transaction that enqueues the operation -- correct local-first
-- bookkeeping, the row no longer names a server position. The drainer then
-- resolved the UID from that nulled row, classified the move as "never
-- uploaded", and marked it done: every archive/move/delete applied locally
-- and silently never reached the server (#289).
--
-- So the queue row itself remembers where the message was when the user
-- acted. NULL on rows enqueued before this migration (and on rows whose
-- message had genuinely never been uploaded): the drainer falls back to the
-- live row exactly as before.
ALTER TABLE operation_queue ADD COLUMN source_uid INTEGER;
ALTER TABLE operation_queue ADD COLUMN source_uid_validity INTEGER;
