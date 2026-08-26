-- Store each address once, not once per header it appears in.
--
-- `recipients` and its indexes measured 56 MB of a 163 MB database -- 34%,
-- and larger than `messages` itself -- across 378,819 rows at 4.6 per message
-- (ADR 0017). Every one of those rows carried the addr-spec *and* its
-- lowercased near-duplicate, so an account that corresponds with a few tens
-- of thousands of people stored those few tens of thousands of strings
-- hundreds of thousands of times, twice each.
--
-- What is per-header stays on the recipient row: `kind`, `position`, and the
-- display `name`, which genuinely differs between messages ("Ada", "Ada
-- Lovelace", "ada"). What is shared is the address itself.
--
-- Note the verbatim spelling is shared too, keyed by the normalized form:
-- `Ada@Example.com` and `ada@example.com` are one correspondent -- which
-- `from:` has always treated as one -- so they become one row, and the first
-- spelling seen is the one kept. That is a real (tiny) loss: a message that
-- addressed someone in unusual case now reads back in the case of whichever
-- message arrived first. It is worth it, and the alternative -- keying on the
-- verbatim form -- would store `Ada@Example.com` and `ada@example.com`
-- separately and lose most of the saving to spelling noise.

CREATE TABLE addresses (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The addr-spec as first seen, for display.
    address             TEXT    NOT NULL,
    -- Lowercased (`EmailAddress::normalized`), and the identity of the row.
    address_normalized  TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_addresses_normalized ON addresses (address_normalized);

-- Every distinct address already in the store. `min(address)` picks one
-- verbatim spelling deterministically rather than whichever the scan reached
-- last, so the migration is reproducible.
INSERT INTO addresses (address, address_normalized)
SELECT min(address), address_normalized FROM recipients GROUP BY address_normalized;

-- Rebuilt rather than altered: dropping two columns and adding a foreign key
-- is a new table either way, and the `message_id XOR draft_id` CHECK has to be
-- carried across intact.
CREATE TABLE recipients_new (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id  INTEGER REFERENCES messages(id) ON DELETE CASCADE,
    draft_id    INTEGER REFERENCES drafts(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL
                        CHECK (kind IN ('from', 'sender', 'reply_to',
                                        'to', 'cc', 'bcc')),
    position    INTEGER NOT NULL DEFAULT 0,
    -- The display name as this header carried it; per-message, so not shared.
    name        TEXT,
    address_id  INTEGER NOT NULL REFERENCES addresses(id),
    CHECK ((message_id IS NOT NULL) <> (draft_id IS NOT NULL))
);

INSERT INTO recipients_new (id, message_id, draft_id, kind, position, name, address_id)
SELECT r.id, r.message_id, r.draft_id, r.kind, r.position, r.name, a.id
  FROM recipients r
  JOIN addresses a ON a.address_normalized = r.address_normalized;

DROP TABLE recipients;
ALTER TABLE recipients_new RENAME TO recipients;

CREATE INDEX idx_recipients_message ON recipients (message_id, kind, position);
-- Partial, unlike its predecessor: `draft_id` is NULL on every row that
-- belongs to a message, which is essentially all of them. The old index
-- covered all 378,819 and was 6 MB of pure NULL (#381).
CREATE INDEX idx_recipients_draft ON recipients (draft_id, kind, position)
    WHERE draft_id IS NOT NULL;
-- `from:`/`to:` search: find the address once, then its rows.
CREATE INDEX idx_recipients_address ON recipients (address_id, kind);
