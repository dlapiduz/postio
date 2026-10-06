-- Payload ownership is independent of a mailbox occurrence. NULL identity
-- allocates independent content; only an adapter guarantee permits sharing.
CREATE TABLE message_contents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    namespace TEXT,
    identity_key TEXT,
    body_text TEXT,
    body_html TEXT,
    body_headers TEXT,
    body_headers_truncated INTEGER NOT NULL DEFAULT 0,
    body_encoding_problems INTEGER NOT NULL DEFAULT 0,
    body_parsed_with INTEGER NOT NULL DEFAULT 0,
    body_line_count INTEGER,
    body_state TEXT NOT NULL DEFAULT 'headers_only',
    raw_blob_id TEXT,
    preview TEXT,
    content_type TEXT,
    text_part_id TEXT,
    text_part_headers TEXT,
    html_part_id TEXT,
    html_part_headers TEXT,
    text_is_flowed INTEGER NOT NULL DEFAULT 0,
    read_receipt_requested INTEGER NOT NULL DEFAULT 0,
    CHECK ((namespace IS NULL AND identity_key IS NULL) OR
           (namespace IS NOT NULL AND identity_key IS NOT NULL AND
            length(namespace) > 0 AND length(identity_key) > 0)),
    UNIQUE (account_id, namespace, identity_key)
);
CREATE INDEX idx_message_contents_raw_blob ON message_contents (raw_blob_id);

-- Cached MIME structure and blob keys survive the occurrence that fetched
-- them. The occurrence's attachment rows remain the reader/action projection.
CREATE TABLE message_content_parts (
    content_id INTEGER NOT NULL REFERENCES message_contents(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    filename TEXT,
    mime_type TEXT NOT NULL,
    size INTEGER NOT NULL,
    mime_content_id TEXT,
    disposition TEXT NOT NULL,
    disposition_raw TEXT,
    part_id TEXT,
    part_headers TEXT,
    blob_id TEXT,
    PRIMARY KEY (content_id, position)
);
CREATE INDEX idx_message_content_parts_blob ON message_content_parts (blob_id);
