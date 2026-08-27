-- The identity flip reaches the queue and the drafts (#543, ADR 0018 Q2).
-- Migration 0023 made `remote_id` the message identity; three other places
-- still carried the IMAP pair and are rewritten the same way, so a store
-- with work in flight crosses the upgrade without losing it:
--
--  * drafts that had located their server copy,
--  * the queue's source snapshot for undrained moves and deletes (#289),
--  * queued discard_draft payloads, whose shape changed with the model.
--
-- The cross-account saga gains its own identity column; the old
-- `confirmed_uid` cannot be backfilled (no generation was stored beside
-- it) and was diagnostic only, so it stays as it is, unread.

UPDATE drafts
SET    remote_id = uid_validity || ':' || uid
WHERE  remote_id IS NULL
  AND  uid IS NOT NULL
  AND  uid_validity IS NOT NULL;

ALTER TABLE operation_queue ADD COLUMN source_remote_id TEXT;

UPDATE operation_queue
SET    source_remote_id = source_uid_validity || ':' || source_uid
WHERE  source_uid IS NOT NULL
  AND  source_uid_validity IS NOT NULL;

UPDATE operation_queue
SET    payload = json_set(
           json_remove(payload, '$.uid', '$.uid_validity'),
           '$.remote_id',
           json_extract(payload, '$.uid_validity') || ':' || json_extract(payload, '$.uid'))
WHERE  json_extract(payload, '$.op') = 'discard_draft'
  AND  json_extract(payload, '$.uid') IS NOT NULL;

ALTER TABLE cross_account_moves ADD COLUMN confirmed_remote_id TEXT;
