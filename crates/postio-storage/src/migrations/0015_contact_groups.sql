-- ADR 0007 Q1/Q3: provenance and suppression on the existing contacts row,
-- plus groups. No behavior change on its own -- nothing yet reads these
-- columns or tables; #473/#474/#475 build on top of this schema.

-- `source` is how the row FIRST appeared, not what it is now. A `mail`-
-- sighted contact the user edits is promoted to `user` by updating this
-- column on the same row, never by inserting a second one.
ALTER TABLE contacts ADD COLUMN source TEXT NOT NULL DEFAULT 'mail'
    CHECK (source IN ('mail', 'user', 'import'));
-- A suppressed contact stays out of autocomplete, the `@` finder and the
-- contact list, but keeps counting sightings -- deleting a `mail`-sourced
-- contact must not let the next sync pass resurrect it (ADR 0007 Q2).
ALTER TABLE contacts ADD COLUMN suppressed INTEGER NOT NULL DEFAULT 0;
-- vCard UID; also CardDAV's key.
ALTER TABLE contacts ADD COLUMN uid TEXT;
-- Every vCard property Postio does not model, preserved verbatim so
-- import/export round-trips losslessly (ADR 0007 Q4).
ALTER TABLE contacts ADD COLUMN vcard_extra TEXT;

-- A group is a named set of contacts, not a saved search -- expanded into
-- member addresses at compose time, never a `To:` header of its own
-- (ADR 0007 Q3).
CREATE TABLE contact_groups (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    -- NULL means the group is shared across accounts, matching contacts.
    account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    uid        TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE contact_group_members (
    group_id   INTEGER NOT NULL REFERENCES contact_groups(id) ON DELETE CASCADE,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, contact_id)
);
