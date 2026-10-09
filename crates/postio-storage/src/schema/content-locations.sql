CREATE INDEX idx_messages_content ON messages (content_id, id);
CREATE INDEX idx_messages_raw_blob ON messages (raw_blob_id);

CREATE TRIGGER trg_messages_content_ai AFTER INSERT ON messages
WHEN new.content_id IS NULL
BEGIN
    INSERT INTO message_contents (account_id, namespace, identity_key, raw_blob_id, preview)
    VALUES (new.account_id, new.content_namespace, new.content_key, new.raw_blob_id, new.preview)
    ON CONFLICT (account_id, namespace, identity_key) DO NOTHING;
    UPDATE messages SET content_id = (
        SELECT id FROM message_contents
         WHERE account_id = new.account_id
           AND namespace IS new.content_namespace AND identity_key IS new.content_key
         ORDER BY id DESC LIMIT 1)
     WHERE id = new.id;
END;

-- Acquiring/changing a backend guarantee does not guess that the old bytes
-- belong to it. Existing independent content may be fetched once again.
CREATE TRIGGER trg_messages_content_identity_au
AFTER UPDATE OF content_namespace, content_key ON messages
WHEN old.content_namespace IS NOT new.content_namespace OR old.content_key IS NOT new.content_key
BEGIN
    INSERT INTO message_contents (account_id, namespace, identity_key, raw_blob_id, preview)
    VALUES (new.account_id, new.content_namespace, new.content_key, new.raw_blob_id, new.preview)
    ON CONFLICT (account_id, namespace, identity_key) DO NOTHING;
    UPDATE messages SET content_id = (
        SELECT id FROM message_contents
         WHERE account_id = new.account_id
           AND namespace IS new.content_namespace AND identity_key IS new.content_key
         ORDER BY id DESC LIMIT 1)
     WHERE id = new.id;
    DELETE FROM message_contents WHERE id = old.content_id
      AND NOT EXISTS (SELECT 1 FROM messages WHERE content_id = old.content_id);
END;

CREATE TRIGGER trg_messages_content_account_bi BEFORE INSERT ON messages
WHEN new.content_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM message_contents WHERE id = new.content_id AND account_id = new.account_id)
BEGIN
    SELECT RAISE(ABORT, 'content belongs to another account');
END;
CREATE TRIGGER trg_messages_content_account_bu BEFORE UPDATE OF content_id, account_id ON messages
WHEN new.content_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM message_contents WHERE id = new.content_id AND account_id = new.account_id)
BEGIN
    SELECT RAISE(ABORT, 'content belongs to another account');
END;

CREATE TRIGGER trg_messages_content_ad AFTER DELETE ON messages
BEGIN
    DELETE FROM message_contents WHERE id = old.content_id
      AND NOT EXISTS (SELECT 1 FROM messages WHERE content_id = old.content_id);
END;

CREATE TRIGGER trg_attachments_content_ai AFTER INSERT ON attachments
WHEN new.message_id IS NOT NULL AND EXISTS (
    SELECT 1 FROM messages WHERE id = new.message_id AND content_namespace IS NOT NULL)
BEGIN
    INSERT INTO message_content_parts
        (content_id, position, filename, mime_type, size, mime_content_id,
         disposition, disposition_raw, part_id, part_headers, blob_id)
    SELECT content_id, new.position, new.filename, new.mime_type, new.size, new.content_id,
           new.disposition, new.disposition_raw, new.part_id, new.part_headers, new.blob_id
      FROM messages WHERE id = new.message_id
    ON CONFLICT (content_id, position) DO UPDATE SET
        filename = excluded.filename, mime_type = excluded.mime_type, size = excluded.size,
        mime_content_id = excluded.mime_content_id, disposition = excluded.disposition,
        disposition_raw = excluded.disposition_raw, part_id = excluded.part_id,
        part_headers = excluded.part_headers,
        blob_id = coalesce(excluded.blob_id, message_content_parts.blob_id);
END;

CREATE TRIGGER trg_attachments_content_blob_au AFTER UPDATE OF blob_id ON attachments
WHEN new.message_id IS NOT NULL AND new.blob_id IS NOT NULL
BEGIN
    UPDATE message_content_parts SET blob_id = new.blob_id
     WHERE content_id = (SELECT content_id FROM messages WHERE id = new.message_id)
       AND part_id = new.part_id AND blob_id IS NOT new.blob_id;
END;
CREATE TRIGGER trg_content_parts_blob_au AFTER UPDATE OF blob_id ON message_content_parts
WHEN new.blob_id IS NOT old.blob_id
BEGIN
    UPDATE attachments SET blob_id = new.blob_id
     WHERE message_id IN (SELECT id FROM messages WHERE content_id = new.content_id)
       AND part_id = new.part_id AND blob_id IS NOT new.blob_id;
END;

-- Parser repairs replace the decoded representation of immutable bytes.
-- A missing derived row is the existing asynchronous indexer's durable queue.
CREATE TRIGGER trg_message_contents_body_index_au
AFTER UPDATE OF body_text, body_html ON message_contents
WHEN old.body_text IS NOT new.body_text OR old.body_html IS NOT new.body_html
BEGIN
    DELETE FROM message_search_bodies WHERE content_id = new.id;
END;
