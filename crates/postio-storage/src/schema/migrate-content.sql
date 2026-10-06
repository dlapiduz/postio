-- Source schema: e9251597, fingerprint -103003987. No identity is inferred.
ALTER TABLE messages ADD COLUMN content_id INTEGER REFERENCES message_contents(id) ON DELETE CASCADE;
ALTER TABLE messages ADD COLUMN content_namespace TEXT;
ALTER TABLE messages ADD COLUMN content_key TEXT;
ALTER TABLE messages ADD COLUMN body_has_headers INTEGER NOT NULL DEFAULT 0;
INSERT INTO message_contents
    (id, account_id, body_text, body_html, body_headers, body_headers_truncated,
     body_encoding_problems, body_parsed_with, body_line_count, body_state, raw_blob_id,
     preview, content_type, text_part_id, text_part_headers, html_part_id,
     html_part_headers, text_is_flowed, read_receipt_requested)
SELECT id, account_id, body_text, body_html, body_headers, body_headers_truncated,
       body_encoding_problems, body_parsed_with, body_line_count, body_state, raw_blob_id,
       preview, content_type, text_part_id, text_part_headers, html_part_id,
       html_part_headers, text_is_flowed, read_receipt_requested FROM messages;
UPDATE messages SET content_id = id, body_has_headers = body_headers IS NOT NULL;
ALTER TABLE messages DROP COLUMN body_text;
ALTER TABLE messages DROP COLUMN body_html;
ALTER TABLE messages DROP COLUMN body_headers;
-- Derived body search is rebuilt once, from the retained content rows.
DROP TABLE message_search_bodies;
CREATE TABLE message_search_bodies (
    content_id INTEGER PRIMARY KEY REFERENCES message_contents(id) ON DELETE CASCADE,
    body_search TEXT NOT NULL
);
