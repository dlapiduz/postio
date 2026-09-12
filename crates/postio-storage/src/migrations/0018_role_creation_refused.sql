-- A server that refused to create a folder for a role, and what it said.
--
-- Spec 003 FR-031. When a reserved role resolves to no folder, discovery
-- creates one (FR-027). A server may refuse -- no permission, the name taken
-- by a non-selectable node, the hierarchy forbidding it -- and a refusal that
-- is not written down is a refusal repeated on every discovery pass, forever,
-- against the user's real server.
--
-- # Why this is not a column on `mailbox_roles`
--
-- That table answers "which folder plays this part", and its `path` is
-- `CHECK (length(path) > 0)` because a mapping to nothing is not a mapping. A
-- refusal has no path by definition: it is the record that there is no folder
-- and that asking for one did not work. Relaxing that CHECK to admit rows
-- meaning the opposite thing would cost the map the one constraint keeping it
-- honest, so this is its own table.
--
-- The two are still related, and the relation is temporal rather than
-- structural: a refusal is cleared when the role's mapping changes or when the
-- folder turns up, because either means the question has been answered by
-- something other than another attempt.
--
-- # What is stored, and what is deliberately not
--
-- `reason` is the server's own words, because "could not create Junk" tells a
-- user nothing they can act on and "Permission denied" tells them to look at
-- their account. It is shown in the account's Mailboxes settings, which is not
-- a log -- Principle VI keeps server messages out of logs, where they would
-- outlive the screen they belong on.
--
-- The role CHECK is the same five as `mailbox_roles`, and for the same reason:
-- `inbox` is never created (RFC 3501 names it and every server has one), and
-- `regular` and `flagged` are not roles a folder is created for.
CREATE TABLE mailbox_role_refusals (
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    role        TEXT    NOT NULL
                        CHECK (role IN ('archive', 'sent', 'drafts', 'trash', 'junk')),
    -- When the server said no. A refusal does not expire on its own: nothing
    -- about waiting makes a permission change, and a retry that happens
    -- because enough time passed is the loop this table exists to stop.
    refused_at  INTEGER NOT NULL,
    -- What the server said, verbatim, for the settings pane to show.
    reason      TEXT    NOT NULL,
    PRIMARY KEY (account_id, role)
) WITHOUT ROWID;
