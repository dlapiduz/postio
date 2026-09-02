-- A fifth draft state: `unconfirmed` (ADR 0021 Decision 3, #674).
--
-- An SMTP session that dies after the payload has begun going out is the one
-- window where the client cannot know whether the message arrived. Postio
-- does not guess: it stops retrying and says so somewhere that persists. That
-- state is neither `sent` (it may not have been) nor `failed` (it may have
-- been), so it needs a name of its own -- "unconfirmed", because it names
-- what is missing and stays true once the confirmation arrives.
--
-- SQLite cannot widen a CHECK in place: there is no `ALTER TABLE ... ALTER
-- COLUMN`, and the constraint is part of the table's own definition. So this
-- is the documented twelve-step rebuild, narrowed to what it needs.
--
-- Every column is copied by name rather than with `INSERT INTO ... SELECT *`.
-- Positional copying is how a table rebuild silently shuffles a user's
-- drafts into the wrong columns the day somebody adds one in the middle, and
-- these rows are mail nobody has sent yet -- the least replaceable thing in
-- the store. `rfc_message_id` is here because #461 added it in 0003; a
-- rebuild that forgot it would drop the reserved `Message-ID` and turn every
-- in-flight retry into a second, distinct message.
--
-- Foreign keys are deferred for the swap rather than disabled: the pragma is
-- a no-op inside a transaction, and this whole file runs in one.

CREATE TABLE drafts_rebuilt (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id              INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- NULL means the account's default identity.
    identity_id             INTEGER REFERENCES identities(id) ON DELETE SET NULL,
    kind                    TEXT    NOT NULL DEFAULT 'new'
                                    CHECK (kind IN ('new', 'reply', 'reply_all', 'forward')),
    in_reply_to_message_id  INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    thread_id               INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    -- The `messages` row this draft was synced back as, once it has one.
    message_id              INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    subject                 TEXT    NOT NULL DEFAULT '',

    -- A draft's body is inline TEXT and uncompressed, unlike a message's, and
    -- for a different reason: autosave writes it on a keystroke. Compressing
    -- per keystroke would spend CPU on a few hundred bytes that are about to
    -- be overwritten. Do not unify these two without reading
    -- `repository/drafts.rs`.
    body_text               TEXT,
    body_html               TEXT,

    state                   TEXT    NOT NULL DEFAULT 'editing'
                                    CHECK (state IN ('editing', 'queued', 'sending',
                                                     'sent', 'failed', 'unconfirmed')),
    -- Populated once the sync engine has appended the draft remotely.
    uid                     INTEGER,
    uid_validity            INTEGER,
    mod_seq                 INTEGER,
    remote_id               TEXT,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    -- 0003's column. Named here so the rebuild carries it; see above.
    rfc_message_id          TEXT
);

INSERT INTO drafts_rebuilt (
    id, account_id, identity_id, kind, in_reply_to_message_id, thread_id,
    message_id, subject, body_text, body_html, state, uid, uid_validity,
    mod_seq, remote_id, created_at, updated_at, rfc_message_id
)
SELECT
    id, account_id, identity_id, kind, in_reply_to_message_id, thread_id,
    message_id, subject, body_text, body_html, state, uid, uid_validity,
    mod_seq, remote_id, created_at, updated_at, rfc_message_id
FROM drafts;

DROP TABLE drafts;
ALTER TABLE drafts_rebuilt RENAME TO drafts;

-- `DROP TABLE` took the indexes with it. Recreated verbatim from 0001: a
-- rebuild that quietly loses them leaves every draft query correct and
-- linear, which is the kind of regression that shows up as "the app got
-- slower" months later rather than as a failure.
CREATE INDEX idx_drafts_account_updated ON drafts (account_id, updated_at DESC);
CREATE INDEX idx_drafts_state ON drafts (state, updated_at);
CREATE INDEX idx_drafts_thread ON drafts (thread_id) WHERE thread_id IS NOT NULL;
CREATE INDEX idx_drafts_message ON drafts (message_id) WHERE message_id IS NOT NULL;
