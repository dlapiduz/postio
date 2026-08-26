-- Named signatures, chosen in the composer independently of the identity.
--
-- Until now a signature was a property of one identity: two columns on
-- `identities`, one signature, no name, no way to pick a different one for a
-- particular message. That covers the common case (a work address signs like
-- work) and nothing else -- not a short form for a quick reply, not a longer
-- one with a disclaimer for external mail, not one shared by two identities
-- that both sign as the same person.
--
-- So a signature becomes a row of its own, owned by the account and named,
-- and the composer can point a draft at any of them without touching the
-- identity it sends from.
--
-- An identity keeps its own signature in the columns it already has: that is
-- what it signs with unless a draft says otherwise, and re-pointing it at a
-- row in the new table would rewrite a working read path to say the same
-- thing. What is new is that the account has a *set*, and the composer can
-- choose from it per message.
--
-- Every existing identity signature is seeded into that set, named after the
-- identity it came from, so the picker is never empty on an account that has
-- been signing mail for months.

CREATE TABLE signatures (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- What the composer's picker shows. Unique per account so the picker
    -- never offers two entries a person cannot tell apart.
    name        TEXT    NOT NULL,
    text        TEXT    NOT NULL,
    html        TEXT,
    -- Preserves the order the picker lists them in.
    position    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_signatures_account ON signatures (account_id, position);

CREATE UNIQUE INDEX idx_signatures_name ON signatures (account_id, name);

-- Carry every existing signature into the set, named after the identity that
-- owned it: `display_name` is what the person called that identity, so it is
-- the name they will recognise in the picker. `GROUP BY` because two
-- identities may share a display name, and the name is unique per account.
INSERT INTO signatures (account_id, name, text, html, position)
SELECT account_id, display_name, signature_text, signature_html, min(position)
FROM identities
WHERE signature_text IS NOT NULL
GROUP BY account_id, display_name;
