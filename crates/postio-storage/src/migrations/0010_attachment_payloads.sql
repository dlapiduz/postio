-- The payload axis: what explains an attachment's bytes, and the backlog of
-- attachments whose bytes are still on the server.
--
-- ADR 0017 splits the backfill in two. The text axis (migration 0008) fetches
-- the sections holding a message's own words and leaves the payloads --
-- ~90% of a mailbox by weight -- where they are. This is the other half:
-- `attachments.blob_id` has been in the schema since 0001 and, until now, has
-- only ever been written on the way *out*, by a composer attaching a file.
-- Nothing in the receive path filled it, so `Attachment::is_downloaded` was
-- false for every message that ever arrived from a server.
--
-- `part_headers` is to a payload what `text_part_headers` is to the text:
-- `BODY[2.1]` returns the part's encoded bytes and none of its headers, and
-- base64 with no `Content-Transfer-Encoding` to explain it is not a PDF.
-- `BODYSTRUCTURE` reported the type, charset and encoding at header-sync
-- time, so keeping the rendered header block costs about twenty bytes a part
-- and saves a `BODY[2.1.MIME]` round trip for every one of them.
--
-- NULL rather than a guess, same convention as 0008: a row synced before this
-- column existed has an honestly unknown encoding, and such a part falls back
-- to the whole-message fetch until its next resync.
ALTER TABLE attachments ADD COLUMN part_headers TEXT;

-- The payload backlog, for `AttachmentPolicy::Eager`.
--
-- `idx_messages_body_state` covers the text lane and deliberately excludes
-- `partial` -- text local, payloads not, which is *settled* as far as that
-- lane is concerned. The payload lane wants exactly the rows that index
-- refuses, so it gets one of its own on the same key.
CREATE INDEX idx_messages_partial
    ON messages (mailbox_id, received_at DESC)
    WHERE body_state = 'partial';

-- Finding the parts of one message that have no bytes yet is a lookup by
-- message, filtered to the rows that are still missing. Partial, so it costs
-- nothing for the attachments that have already landed.
CREATE INDEX idx_attachments_pending
    ON attachments (message_id)
    WHERE blob_id IS NULL AND part_id IS NOT NULL;
