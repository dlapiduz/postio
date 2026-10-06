-- Refresh the small read projection without changing location identity or flags.
CREATE TRIGGER trg_content_projection_contents_au AFTER UPDATE ON message_contents

BEGIN
    DELETE FROM attachments WHERE message_id IN (SELECT id FROM messages WHERE content_id = new.id)
      AND EXISTS (SELECT 1 FROM message_contents WHERE id = new.id AND namespace IS NOT NULL AND body_state IN ('partial','full'))
      AND position NOT IN (SELECT position FROM message_content_parts WHERE content_id = new.id);
    INSERT INTO attachments (message_id, position, filename, mime_type, size, content_id, disposition, disposition_raw, part_id, part_headers, blob_id)
    SELECT m.id, p.position, p.filename, p.mime_type, p.size, p.mime_content_id, p.disposition, p.disposition_raw, p.part_id, p.part_headers, p.blob_id
      FROM messages m JOIN message_content_parts p ON p.content_id = m.content_id
     WHERE m.content_id = new.id AND EXISTS (SELECT 1 FROM message_contents WHERE id = new.id AND namespace IS NOT NULL AND body_state IN ('partial','full'))
       AND NOT EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id AND a.position = p.position);
    UPDATE attachments SET blob_id = (
        SELECT p.blob_id FROM message_content_parts p
         WHERE p.content_id = new.id AND p.position = attachments.position)
     WHERE message_id IN (SELECT id FROM messages WHERE content_id = new.id) AND EXISTS (SELECT 1 FROM message_contents WHERE id = new.id AND namespace IS NOT NULL AND body_state IN ('partial','full'));
    UPDATE messages SET
        body_headers_truncated = (SELECT body_headers_truncated FROM message_contents WHERE id = new.id),
        body_encoding_problems = (SELECT body_encoding_problems FROM message_contents WHERE id = new.id),
        body_parsed_with = (SELECT body_parsed_with FROM message_contents WHERE id = new.id),
        body_line_count = (SELECT body_line_count FROM message_contents WHERE id = new.id),
        content_type = coalesce((SELECT content_type FROM message_contents WHERE id = new.id), content_type),
        text_part_id = coalesce((SELECT text_part_id FROM message_contents WHERE id = new.id), text_part_id),
        text_part_headers = coalesce((SELECT text_part_headers FROM message_contents WHERE id = new.id), text_part_headers),
        html_part_id = coalesce((SELECT html_part_id FROM message_contents WHERE id = new.id), html_part_id),
        html_part_headers = coalesce((SELECT html_part_headers FROM message_contents WHERE id = new.id), html_part_headers),
        text_is_flowed = (SELECT text_is_flowed FROM message_contents WHERE id = new.id),
        read_receipt_requested = (SELECT read_receipt_requested FROM message_contents WHERE id = new.id),
        body_has_headers = (SELECT body_headers IS NOT NULL FROM message_contents WHERE id = new.id),
        raw_blob_id = coalesce((SELECT raw_blob_id FROM message_contents WHERE id = new.id), raw_blob_id),
        preview = coalesce(preview, (SELECT preview FROM message_contents WHERE id = new.id)),
        has_attachments = EXISTS (SELECT 1 FROM attachments WHERE message_id = messages.id),
        body_state = CASE
            WHEN (SELECT body_state FROM message_contents WHERE id = new.id) IN ('partial','full')
            THEN CASE
                WHEN content_namespace IS NULL THEN (SELECT body_state FROM message_contents WHERE id = new.id)
                WHEN EXISTS (SELECT 1 FROM attachments WHERE message_id = messages.id AND blob_id IS NULL) THEN 'partial'
                ELSE 'full' END
            WHEN content_namespace IS NOT NULL THEN 'headers_only'
            ELSE body_state END
     WHERE content_id = new.id;
END;

-- Refresh the small read projection without changing location identity or flags.
CREATE TRIGGER trg_content_projection_location_au AFTER UPDATE OF content_id ON messages
WHEN new.content_namespace IS NOT NULL
BEGIN
    DELETE FROM attachments WHERE message_id IN (SELECT id FROM messages WHERE id = new.id)
      AND EXISTS (SELECT 1 FROM message_contents WHERE id = new.content_id AND namespace IS NOT NULL AND body_state IN ('partial','full'))
      AND position NOT IN (SELECT position FROM message_content_parts WHERE content_id = new.content_id);
    INSERT INTO attachments (message_id, position, filename, mime_type, size, content_id, disposition, disposition_raw, part_id, part_headers, blob_id)
    SELECT m.id, p.position, p.filename, p.mime_type, p.size, p.mime_content_id, p.disposition, p.disposition_raw, p.part_id, p.part_headers, p.blob_id
      FROM messages m JOIN message_content_parts p ON p.content_id = m.content_id
     WHERE m.id = new.id AND EXISTS (SELECT 1 FROM message_contents WHERE id = new.content_id AND namespace IS NOT NULL AND body_state IN ('partial','full'))
       AND NOT EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = m.id AND a.position = p.position);
    UPDATE attachments SET blob_id = (
        SELECT p.blob_id FROM message_content_parts p
         WHERE p.content_id = new.content_id AND p.position = attachments.position)
     WHERE message_id IN (SELECT id FROM messages WHERE id = new.id) AND EXISTS (SELECT 1 FROM message_contents WHERE id = new.content_id AND namespace IS NOT NULL AND body_state IN ('partial','full'));
    UPDATE messages SET
        body_headers_truncated = (SELECT body_headers_truncated FROM message_contents WHERE id = new.content_id),
        body_encoding_problems = (SELECT body_encoding_problems FROM message_contents WHERE id = new.content_id),
        body_parsed_with = (SELECT body_parsed_with FROM message_contents WHERE id = new.content_id),
        body_line_count = (SELECT body_line_count FROM message_contents WHERE id = new.content_id),
        content_type = coalesce((SELECT content_type FROM message_contents WHERE id = new.content_id), content_type),
        text_part_id = coalesce((SELECT text_part_id FROM message_contents WHERE id = new.content_id), text_part_id),
        text_part_headers = coalesce((SELECT text_part_headers FROM message_contents WHERE id = new.content_id), text_part_headers),
        html_part_id = coalesce((SELECT html_part_id FROM message_contents WHERE id = new.content_id), html_part_id),
        html_part_headers = coalesce((SELECT html_part_headers FROM message_contents WHERE id = new.content_id), html_part_headers),
        text_is_flowed = (SELECT text_is_flowed FROM message_contents WHERE id = new.content_id),
        read_receipt_requested = (SELECT read_receipt_requested FROM message_contents WHERE id = new.content_id),
        body_has_headers = (SELECT body_headers IS NOT NULL FROM message_contents WHERE id = new.content_id),
        raw_blob_id = coalesce((SELECT raw_blob_id FROM message_contents WHERE id = new.content_id), raw_blob_id),
        preview = coalesce(preview, (SELECT preview FROM message_contents WHERE id = new.content_id)),
        has_attachments = EXISTS (SELECT 1 FROM attachments WHERE message_id = messages.id),
        body_state = CASE
            WHEN (SELECT body_state FROM message_contents WHERE id = new.content_id) IN ('partial','full')
            THEN CASE
                WHEN content_namespace IS NULL THEN (SELECT body_state FROM message_contents WHERE id = new.content_id)
                WHEN EXISTS (SELECT 1 FROM attachments WHERE message_id = messages.id AND blob_id IS NULL) THEN 'partial'
                ELSE 'full' END
            WHEN content_namespace IS NOT NULL THEN 'headers_only'
            ELSE body_state END
     WHERE id = new.id;
END;
