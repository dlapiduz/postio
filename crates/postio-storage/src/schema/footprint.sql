-- What an account's mail weighs, kept rather than counted (#1508).
--
-- The settings panel shows every account's weight and the privacy pane how
-- many messages asked for a read receipt. Both were aggregates over every
-- message the account holds, read each time the panel opened -- an
-- interaction, budgeted at 16 ms, paying a walk of the whole mailbox. ADR
-- 0040's rule: an aggregate a surface asks for is a maintained answer.
--
-- Two levels, so that no trigger here ever looks at another table's rows:
-- `attachments` keeps its message's `attachment_bytes` and
-- `attachment_bytes_unfetched` (those with no blob yet), and `messages`
-- keeps its account's row from its own columns. A message's delete then
-- carries its attachments' bytes in `old`, however its attachment rows are
-- removed around it. A message counts while `deleted_locally = 0`, as it did
-- in the aggregates this replaces. Decrements are clamped at zero, as the
-- mailbox counts are: a figure that drifted degrades to a wrong number, not
-- to a negative one read back as a byte count of 2^64.
--
-- What is local: a message's whole size once every part is here, and only
-- its text while its payloads are still on the server (ADR 0017).

CREATE TABLE account_footprints (
    account_id        INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    messages          INTEGER NOT NULL DEFAULT 0,
    total_bytes       INTEGER NOT NULL DEFAULT 0,
    attachment_bytes  INTEGER NOT NULL DEFAULT 0,
    local_bytes       INTEGER NOT NULL DEFAULT 0,
    read_receipts     INTEGER NOT NULL DEFAULT 0
);

CREATE TRIGGER trg_footprint_accounts_ai AFTER INSERT ON accounts
BEGIN
    INSERT INTO account_footprints (account_id) VALUES (new.id);
END;

CREATE TRIGGER trg_footprint_messages_ai AFTER INSERT ON messages
WHEN new.deleted_locally = 0
BEGIN
    UPDATE account_footprints SET
        messages = messages + 1,
        total_bytes = total_bytes + new.size,
        attachment_bytes = attachment_bytes + new.attachment_bytes,
        local_bytes = local_bytes + CASE new.body_state
            WHEN 'full' THEN new.size
            WHEN 'partial' THEN new.size - new.attachment_bytes_unfetched
            ELSE 0 END,
        read_receipts = read_receipts + (new.read_receipt_requested = 1)
     WHERE account_id = new.account_id;
END;

CREATE TRIGGER trg_footprint_messages_ad AFTER DELETE ON messages
WHEN old.deleted_locally = 0
BEGIN
    UPDATE account_footprints SET
        messages = max(messages - 1, 0),
        total_bytes = max(total_bytes - old.size, 0),
        attachment_bytes = max(attachment_bytes - old.attachment_bytes, 0),
        local_bytes = max(local_bytes - CASE old.body_state
            WHEN 'full' THEN old.size
            WHEN 'partial' THEN old.size - old.attachment_bytes_unfetched
            ELSE 0 END, 0),
        read_receipts = max(read_receipts - (old.read_receipt_requested = 1), 0)
     WHERE account_id = old.account_id;
END;

CREATE TRIGGER trg_footprint_messages_au
AFTER UPDATE OF account_id, deleted_locally, size, body_state, read_receipt_requested,
                attachment_bytes, attachment_bytes_unfetched ON messages
WHEN old.account_id IS NOT new.account_id
  OR old.deleted_locally IS NOT new.deleted_locally
  OR old.size IS NOT new.size
  OR old.body_state IS NOT new.body_state
  OR old.read_receipt_requested IS NOT new.read_receipt_requested
  OR old.attachment_bytes IS NOT new.attachment_bytes
  OR old.attachment_bytes_unfetched IS NOT new.attachment_bytes_unfetched
BEGIN
    UPDATE account_footprints SET
        messages = max(messages - 1, 0),
        total_bytes = max(total_bytes - old.size, 0),
        attachment_bytes = max(attachment_bytes - old.attachment_bytes, 0),
        local_bytes = max(local_bytes - CASE old.body_state
            WHEN 'full' THEN old.size
            WHEN 'partial' THEN old.size - old.attachment_bytes_unfetched
            ELSE 0 END, 0),
        read_receipts = max(read_receipts - (old.read_receipt_requested = 1), 0)
     WHERE account_id = old.account_id AND old.deleted_locally = 0;
    UPDATE account_footprints SET
        messages = messages + 1,
        total_bytes = total_bytes + new.size,
        attachment_bytes = attachment_bytes + new.attachment_bytes,
        local_bytes = local_bytes + CASE new.body_state
            WHEN 'full' THEN new.size
            WHEN 'partial' THEN new.size - new.attachment_bytes_unfetched
            ELSE 0 END,
        read_receipts = read_receipts + (new.read_receipt_requested = 1)
     WHERE account_id = new.account_id AND new.deleted_locally = 0;
END;

CREATE TRIGGER trg_footprint_attachments_ai AFTER INSERT ON attachments
WHEN new.message_id IS NOT NULL
BEGIN
    UPDATE messages SET
        attachment_bytes = attachment_bytes + new.size,
        attachment_bytes_unfetched = attachment_bytes_unfetched
            + CASE WHEN new.blob_id IS NULL THEN new.size ELSE 0 END
     WHERE id = new.message_id;
END;

CREATE TRIGGER trg_footprint_attachments_ad AFTER DELETE ON attachments
WHEN old.message_id IS NOT NULL
BEGIN
    UPDATE messages SET
        attachment_bytes = max(attachment_bytes - old.size, 0),
        attachment_bytes_unfetched = max(attachment_bytes_unfetched
            - CASE WHEN old.blob_id IS NULL THEN old.size ELSE 0 END, 0)
     WHERE id = old.message_id;
END;

CREATE TRIGGER trg_footprint_attachments_au AFTER UPDATE OF message_id, size, blob_id ON attachments
WHEN old.message_id IS NOT new.message_id
  OR old.size IS NOT new.size
  OR (old.blob_id IS NULL) <> (new.blob_id IS NULL)
BEGIN
    UPDATE messages SET
        attachment_bytes = max(attachment_bytes - old.size, 0),
        attachment_bytes_unfetched = max(attachment_bytes_unfetched
            - CASE WHEN old.blob_id IS NULL THEN old.size ELSE 0 END, 0)
     WHERE id = old.message_id;
    UPDATE messages SET
        attachment_bytes = attachment_bytes + new.size,
        attachment_bytes_unfetched = attachment_bytes_unfetched
            + CASE WHEN new.blob_id IS NULL THEN new.size ELSE 0 END
     WHERE id = new.message_id;
END;
