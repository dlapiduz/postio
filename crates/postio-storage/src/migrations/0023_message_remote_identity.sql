-- Remote identity (#543, ADR 0018 Q2): `remote_id` becomes the
-- backend-neutral message identity. The column has existed since the
-- initial schema; what changes is that it is now *the* identity the
-- engine addresses messages by, so every row an IMAP server has named
-- gets one, derived the way the IMAP adapter derives it from now on:
-- the generation and the uid, joined. Rows the server never named --
-- locally composed, or seen before UIDVALIDITY was recorded -- keep
-- NULL: an invented identity is worse than none.

UPDATE messages
SET    remote_id = uid_validity || ':' || uid
WHERE  remote_id IS NULL
  AND  uid IS NOT NULL
  AND  uid_validity IS NOT NULL;

-- The engine's lookup is by mailbox and identity.
CREATE INDEX idx_messages_mailbox_remote_id
    ON messages (mailbox_id, remote_id);
