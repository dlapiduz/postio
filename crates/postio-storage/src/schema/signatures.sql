
CREATE TABLE public_keys (
    fingerprint  TEXT    PRIMARY KEY,
    kind         TEXT    NOT NULL CHECK (kind IN ('openpgp', 'x509')),
    keyblock     BLOB    NOT NULL,
    primary_uid  TEXT,
    revoked      INTEGER NOT NULL DEFAULT 0,
    first_seen   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL
);
-- A key with no source left and no acceptance is forgotten when a source is switched off.
CREATE TABLE public_key_sources (
    fingerprint  TEXT    NOT NULL REFERENCES public_keys(fingerprint) ON DELETE CASCADE,
    source       TEXT    NOT NULL CHECK (source IN ('autocrypt', 'attachment', 'file', 'gnupg')),
    message_id   INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    seen_at      INTEGER NOT NULL,
    PRIMARY KEY (fingerprint, source)
);
CREATE TABLE public_key_addresses (
    fingerprint  TEXT    NOT NULL REFERENCES public_keys(fingerprint) ON DELETE CASCADE,
    address_id   INTEGER NOT NULL REFERENCES addresses(id),
    PRIMARY KEY (fingerprint, address_id)
);
CREATE INDEX idx_public_key_addresses_address ON public_key_addresses (address_id);
-- On the contact, where what the person decided about a correspondent lives (ADR 0048).
CREATE TABLE contact_accepted_keys (
    contact_id   INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    address_id   INTEGER NOT NULL REFERENCES addresses(id),
    fingerprint  TEXT    NOT NULL,
    accepted_at  INTEGER NOT NULL,
    PRIMARY KEY (contact_id, address_id)
);
CREATE INDEX idx_contact_accepted_keys_address ON contact_accepted_keys (address_id);
CREATE TABLE signature_verdicts (
    message_id   INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    format       TEXT    NOT NULL CHECK (format IN ('pgp_mime', 'inline_pgp', 'smime', 'encrypted')),
    scope        TEXT    NOT NULL CHECK (scope IN ('whole', 'part', 'inline', 'encrypted')),
    status       TEXT    NOT NULL CHECK (status IN ('good', 'bad', 'cant_check', 'not_here_yet', 'revoked')),
    reason       TEXT,
    fingerprint  TEXT,
    checked_at   INTEGER NOT NULL
);
