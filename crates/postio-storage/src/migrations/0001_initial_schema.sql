-- The Postio store, whole.
--
-- This file *is* the schema. It is not the first of a series of corrections --
-- it is what a reader should read to find out what is true, and the only
-- migration a fresh store applies.
--
-- It replaces twenty-five numbered migrations that had accumulated since the
-- store was first written. ADR 0020 records why that history was discarded
-- rather than extended: every one of those files was a fact about a past
-- mistake, none was a fact about the schema, and collapsing them was a licence
-- available exactly once, while the installed user count was one. The chain
-- starts again from here for anybody who installs Postio after this.
--
-- Conventions used throughout:
--
--   * Every entity id is `INTEGER PRIMARY KEY AUTOINCREMENT`, matching the
--     model's newtype ids.
--   * Times are milliseconds since the Unix epoch, INTEGER.
--   * Booleans are INTEGER 0/1.
--   * A closed set of strings is spelled as a CHECK, so the database refuses a
--     value the model cannot name.
--   * Foreign keys cascade where the child is meaningless without its parent
--     and SET NULL where it survives alone.

------------------------------------------------------------------------------
-- Accounts and identities
------------------------------------------------------------------------------

CREATE TABLE accounts (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    display_name         TEXT    NOT NULL,
    -- The account's primary address, verbatim, plus the display name it
    -- carried. Together these rebuild an `EmailAddress`.
    address              TEXT    NOT NULL,
    address_name         TEXT,
    incoming_host        TEXT    NOT NULL,
    incoming_port        INTEGER NOT NULL,
    incoming_security    TEXT    NOT NULL DEFAULT 'tls'
                                 CHECK (incoming_security IN ('none', 'starttls', 'tls')),
    incoming_username    TEXT    NOT NULL,
    outgoing_host        TEXT    NOT NULL,
    outgoing_port        INTEGER NOT NULL,
    outgoing_security    TEXT    NOT NULL DEFAULT 'starttls'
                                 CHECK (outgoing_security IN ('none', 'starttls', 'tls')),
    outgoing_username    TEXT    NOT NULL,
    auth_method          TEXT    NOT NULL DEFAULT 'password'
                                 CHECK (auth_method IN ('password', 'app_password',
                                                        'oauth2', 'xoauth2')),

    -- Which protocol family talks to this account. This names a backend, never
    -- a provider: providers are data, in the preset table.
    backend              TEXT    NOT NULL DEFAULT 'imap',
    -- JMAP's session resource, for `backend = 'jmap'`.
    jmap_session_url     TEXT,

    -- OAuth endpoints, from the provider preset table rather than from a
    -- constant in the code. Tokens themselves are in the keyring, never here.
    oauth_client_id      TEXT,
    oauth_token_url      TEXT,
    oauth_authorize_url  TEXT,
    oauth_scopes         TEXT,

    default_signature_id INTEGER REFERENCES signatures(id) ON DELETE SET NULL,
    -- Set while the account is being torn down, so a half-removed account is
    -- never offered as a live one.
    pending_deletion     INTEGER NOT NULL DEFAULT 0,
    enabled              INTEGER NOT NULL DEFAULT 1,
    created_at           INTEGER NOT NULL
);

CREATE TABLE identities (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id        INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    display_name      TEXT    NOT NULL,
    address           TEXT    NOT NULL,
    address_name      TEXT,
    reply_to_address  TEXT,
    reply_to_name     TEXT,
    signature_text    TEXT,
    signature_html    TEXT,
    is_default        INTEGER NOT NULL DEFAULT 0,
    -- Preserves the order of `Account::identities`, which the picker shows.
    position          INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_identities_account ON identities (account_id, position);
CREATE UNIQUE INDEX idx_identities_one_default
    ON identities (account_id) WHERE is_default = 1;

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

------------------------------------------------------------------------------
-- Mailboxes and sync state
------------------------------------------------------------------------------

CREATE TABLE mailboxes (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id         INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    parent_id          INTEGER REFERENCES mailboxes(id) ON DELETE SET NULL,
    name               TEXT    NOT NULL,
    path               TEXT    NOT NULL,
    -- The server's hierarchy delimiter, a single character, or NULL for a flat
    -- namespace.
    delimiter          TEXT    CHECK (delimiter IS NULL OR length(delimiter) = 1),
    role               TEXT    NOT NULL DEFAULT 'regular'
                               CHECK (role IN ('inbox', 'archive', 'sent', 'drafts',
                                               'trash', 'junk', 'flagged', 'regular')),
    selectable         INTEGER NOT NULL DEFAULT 1,
    subscribed         INTEGER NOT NULL DEFAULT 1,
    -- Cached counts, so the sidebar never counts rows. Maintained by the
    -- triggers at the foot of this file.
    total_count        INTEGER NOT NULL DEFAULT 0,
    unread_count       INTEGER NOT NULL DEFAULT 0,
    flagged_count      INTEGER NOT NULL DEFAULT 0,
    snoozed_count      INTEGER NOT NULL DEFAULT 0,
    -- A mailbox the user has told the backfill to leave alone (ADR 0016).
    backfill_excluded  INTEGER NOT NULL DEFAULT 0,
    signature_id       INTEGER REFERENCES signatures(id) ON DELETE SET NULL,
    last_synced_at     INTEGER
);

CREATE UNIQUE INDEX idx_mailboxes_account_path ON mailboxes (account_id, path);
CREATE INDEX idx_mailboxes_account_role ON mailboxes (account_id, role);
CREATE INDEX idx_mailboxes_parent ON mailboxes (parent_id);

CREATE TABLE sync_state (
    mailbox_id         INTEGER PRIMARY KEY REFERENCES mailboxes(id) ON DELETE CASCADE,
    account_id         INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- Generation of the UID space. A change invalidates every cached UID and
    -- forces a full resync of this mailbox.
    uid_validity       INTEGER,
    -- The UID the server said it will assign next.
    uid_next           INTEGER,
    -- Highest MODSEQ seen, for QRESYNC incremental resync.
    highest_mod_seq    INTEGER,
    last_full_sync_at  INTEGER,
    last_seen_at       INTEGER
);

CREATE INDEX idx_sync_state_account ON sync_state (account_id);

------------------------------------------------------------------------------
-- Threads
------------------------------------------------------------------------------

CREATE TABLE threads (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id       INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- Normalized subject of the root message (`normalize_subject`).
    subject          TEXT,
    message_count    INTEGER NOT NULL DEFAULT 0,
    unread_count     INTEGER NOT NULL DEFAULT 0,
    has_attachments  INTEGER NOT NULL DEFAULT 0,
    is_flagged       INTEGER NOT NULL DEFAULT 0,
    first_at         INTEGER NOT NULL DEFAULT 0,
    last_at          INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_threads_account_last_at ON threads (account_id, last_at DESC, id DESC);
CREATE INDEX idx_threads_account_subject ON threads (account_id, subject);
-- The unified view sorts across accounts, so it needs the account-free forms.
CREATE INDEX idx_threads_last_at ON threads (last_at DESC, id DESC);
CREATE INDEX idx_threads_subject ON threads (subject) WHERE subject IS NOT NULL;

CREATE TABLE thread_links (
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- Normalized by `RfcMessageId`: trimmed and angle-bracketed, so a lookup
    -- matches without further munging. Compared case-insensitively, because
    -- the wild does not agree on case.
    rfc_message_id TEXT    NOT NULL,
    thread_id      INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    PRIMARY KEY (account_id, rfc_message_id)
) WITHOUT ROWID;

CREATE UNIQUE INDEX idx_thread_links_lookup
    ON thread_links (account_id, rfc_message_id COLLATE NOCASE);
CREATE INDEX idx_thread_links_thread ON thread_links (thread_id);

------------------------------------------------------------------------------
-- The compression dictionary for message bodies
------------------------------------------------------------------------------

-- A trained zstd dictionary, stored as a row (ADR 0020).
--
-- Bodies compress about 1.57x on their own and about 2.19x against a
-- dictionary trained on the mailbox they came from, because mail from one
-- correspondence is full of the same signatures, the same quoted headers and
-- the same boilerplate.
--
-- **In a row, deliberately.** A dictionary held as a file beside the database
-- would be a new way to lose mail: lose it and every body written against it
-- is unreadable. Here it is backed up, encrypted and restored with the data it
-- decodes, and cannot go missing independently.
CREATE TABLE body_dictionaries (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    dictionary    BLOB    NOT NULL,
    -- What this was trained from, so a later pass can tell whether the corpus
    -- has grown enough to be worth training again.
    sample_count  INTEGER NOT NULL,
    sample_bytes  INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);

------------------------------------------------------------------------------
-- Messages
------------------------------------------------------------------------------

CREATE TABLE messages (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id              INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mailbox_id              INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
    thread_id               INTEGER REFERENCES threads(id) ON DELETE SET NULL,

    -- RFC 5322 identity. Normalized by `RfcMessageId`: trimmed and always
    -- angle-bracketed, so lookups match without further munging.
    rfc_message_id          TEXT,
    in_reply_to             TEXT,
    -- `References`, oldest ancestor first, space separated. A list column
    -- rather than a join table: it is only ever read whole, by the threading
    -- pass, and never searched.
    reference_ids           TEXT    NOT NULL DEFAULT '',

    subject                 TEXT,
    -- `Subject` with Re:/Fwd: prefixes stripped, for subject-based threading.
    normalized_subject      TEXT,
    -- The `Date` header, as claimed by the sender; may be absent or a lie.
    date                    INTEGER,
    -- When the server received it. Always known; this is the list sort key.
    received_at             INTEGER NOT NULL,
    -- `List-Id`, for the mailing-list filters.
    list_id                 TEXT,
    -- The top-level `Content-Type`.
    content_type            TEXT,

    -- A short plain-text snippet for the list.
    preview                 TEXT,
    size                    INTEGER NOT NULL DEFAULT 0,

    -- Canonical flag spellings, space separated, in `FlagSet` order and with
    -- `\Recent` already stripped (`FlagSet::persistable`).
    flags                   TEXT    NOT NULL DEFAULT '',
    -- Denormalized from `flags` so the list and its filters never parse a
    -- string. Repositories write both in the same statement.
    seen                    INTEGER NOT NULL DEFAULT 0,
    flagged                 INTEGER NOT NULL DEFAULT 0,
    answered                INTEGER NOT NULL DEFAULT 0,
    draft                   INTEGER NOT NULL DEFAULT 0,
    -- The IMAP `\Deleted` flag (marked for expunge on the server). Postio's own
    -- local delete is `deleted_locally`.
    deleted                 INTEGER NOT NULL DEFAULT 0,
    has_attachments         INTEGER NOT NULL DEFAULT 0,
    -- Hidden until this time, and counted as snoozed rather than as unread.
    snoozed_until           INTEGER,

    -- Server identifiers. Protocol-neutral: IMAP fills the first three, another
    -- backend may fill only `remote_id`. A `uid` is meaningless without the
    -- `uid_validity` it was seen under.
    uid                     INTEGER,
    uid_validity            INTEGER,
    mod_seq                 INTEGER,
    remote_id               TEXT,

    -- Local synchronization state. These are what tell the sync engine the
    -- local row is ahead of the server.
    body_state              TEXT    NOT NULL DEFAULT 'not_fetched'
                                    CHECK (body_state IN ('not_fetched', 'headers_only',
                                                          'partial', 'full')),
    flags_dirty             INTEGER NOT NULL DEFAULT 0,
    has_pending_operations  INTEGER NOT NULL DEFAULT 0,
    -- Hidden locally pending a remote delete or move; the list filters on this.
    deleted_locally         INTEGER NOT NULL DEFAULT 0,
    last_synced_at          INTEGER,

    -- The message's decoded text, zstd-compressed, written and read by
    -- `MessageRepository` (ADR 0020). Nothing above the storage layer knows
    -- these are compressed, or that they are here rather than in a file.
    --
    -- NULL means "no such part". A part that exists and is empty is a
    -- zero-length value, which is a different fact and one the reading pane
    -- distinguishes.
    --
    -- These are the most sensitive bytes in the product, and SQLCipher is what
    -- encrypts them (#300). A file per body would leak its size and its mtime
    -- even when encrypted; a row leaks neither.
    body_text               BLOB,
    body_html               BLOB,
    -- The full header block, preserved for display and later reparsing.
    body_headers            BLOB,
    -- Which dictionary the three values above were compressed against, or NULL
    -- for values compressed on their own. Per row rather than per value,
    -- because all three are written in one call against one dictionary.
    --
    -- No cascade and no SET NULL on purpose: deleting a dictionary a row still
    -- names would make that row's body unreadable, so the database refuses.
    body_dictionary_id      INTEGER REFERENCES body_dictionaries(id) ON DELETE RESTRICT,

    -- The raw RFC 5322 source, in the content-addressed blob store. Bodies are
    -- not there: the blob store holds attachments and raw messages, which are
    -- large, stream, and are worth deduplicating. Bodies are none of those
    -- (ADR 0020).
    raw_blob_id             TEXT,

    -- Where the text and HTML parts sit in the MIME structure, and the headers
    -- those parts carried, so a part can be refetched by path.
    text_part_id            TEXT,
    text_part_headers       TEXT,
    html_part_id            TEXT,
    html_part_headers       TEXT
);

CREATE INDEX idx_messages_list ON messages (mailbox_id, received_at DESC, id DESC);
CREATE INDEX idx_messages_account_list ON messages (account_id, received_at DESC, id DESC);
-- The unified view: every account at once, newest first.
CREATE INDEX idx_messages_recency ON messages (received_at DESC, id DESC);
CREATE INDEX idx_messages_thread ON messages (thread_id, received_at, id);
CREATE INDEX idx_messages_thread_mailbox
    ON messages (thread_id, mailbox_id, received_at DESC, id DESC);
CREATE INDEX idx_messages_rfc_message_id
    ON messages (account_id, rfc_message_id) WHERE rfc_message_id IS NOT NULL;
CREATE INDEX idx_messages_in_reply_to
    ON messages (account_id, in_reply_to) WHERE in_reply_to IS NOT NULL;
CREATE INDEX idx_messages_flagged
    ON messages (account_id, received_at DESC, id DESC) WHERE flagged = 1;
CREATE INDEX idx_messages_list_id ON messages (account_id, list_id) WHERE list_id IS NOT NULL;
CREATE INDEX idx_messages_mod_seq ON messages (mailbox_id, mod_seq);
CREATE UNIQUE INDEX idx_messages_uid
    ON messages (mailbox_id, uid_validity, uid) WHERE uid IS NOT NULL;
CREATE INDEX idx_messages_mailbox_remote_id ON messages (mailbox_id, remote_id);
-- The backfill's queue: what still has no body, newest first.
CREATE INDEX idx_messages_body_state
    ON messages (mailbox_id, received_at DESC)
    WHERE body_state IN ('not_fetched', 'headers_only');
-- And what has a body but is still missing payloads.
CREATE INDEX idx_messages_partial
    ON messages (mailbox_id, received_at DESC)
    WHERE body_state = 'partial';

------------------------------------------------------------------------------
-- Attachments
------------------------------------------------------------------------------

CREATE TABLE attachments (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id        INTEGER REFERENCES messages(id) ON DELETE CASCADE,
    -- An attachment added to a draft has no message yet; the model spells that
    -- `MessageId::UNASSIGNED`, storage spells it NULL.
    draft_id          INTEGER REFERENCES drafts(id) ON DELETE CASCADE,
    position          INTEGER NOT NULL DEFAULT 0,
    filename          TEXT,
    mime_type         TEXT    NOT NULL,
    size              INTEGER NOT NULL DEFAULT 0,
    content_id        TEXT,
    disposition       TEXT    NOT NULL DEFAULT 'attachment'
                              CHECK (disposition IN ('inline', 'attachment', 'other')),
    -- The verbatim disposition for `Disposition::Other`, so it round-trips.
    disposition_raw   TEXT,
    -- MIME part path within the message, e.g. `2.1`, for a lazy fetch.
    part_id           TEXT,
    -- The headers that part carried.
    part_headers      TEXT,
    -- Blob store key; NULL until the bytes have been downloaded. Payloads stay
    -- in the blob store: they are large, they stream, and the same PDF really
    -- does arrive five times.
    blob_id           TEXT,
    CHECK ((message_id IS NOT NULL) <> (draft_id IS NOT NULL)),
    CHECK (disposition <> 'other' OR disposition_raw IS NOT NULL)
);

CREATE INDEX idx_attachments_message ON attachments (message_id, position);
CREATE INDEX idx_attachments_draft ON attachments (draft_id, position);
CREATE INDEX idx_attachments_filename ON attachments (filename) WHERE filename IS NOT NULL;
CREATE INDEX idx_attachments_blob ON attachments (blob_id) WHERE blob_id IS NOT NULL;
-- Payloads known from BODYSTRUCTURE but not downloaded: the payload axis's queue.
CREATE INDEX idx_attachments_pending
    ON attachments (message_id)
    WHERE blob_id IS NULL AND part_id IS NOT NULL;

------------------------------------------------------------------------------
-- Labels
------------------------------------------------------------------------------

CREATE TABLE labels (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    -- Optional hex colour, e.g. `#5980a6`.
    color       TEXT
);

CREATE UNIQUE INDEX idx_labels_account_name ON labels (account_id, name COLLATE NOCASE);

CREATE TABLE message_labels (
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label_id    INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, label_id)
) WITHOUT ROWID;

CREATE INDEX idx_message_labels_label ON message_labels (label_id, message_id);

------------------------------------------------------------------------------
-- Drafts
------------------------------------------------------------------------------

CREATE TABLE drafts (
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
                                                     'sent', 'failed')),
    -- Populated once the sync engine has appended the draft remotely.
    uid                     INTEGER,
    uid_validity            INTEGER,
    mod_seq                 INTEGER,
    remote_id               TEXT,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);

CREATE INDEX idx_drafts_account_updated ON drafts (account_id, updated_at DESC);
CREATE INDEX idx_drafts_state ON drafts (state, updated_at);
CREATE INDEX idx_drafts_thread ON drafts (thread_id) WHERE thread_id IS NOT NULL;
CREATE INDEX idx_drafts_message ON drafts (message_id) WHERE message_id IS NOT NULL;

------------------------------------------------------------------------------
-- Addresses and recipients
------------------------------------------------------------------------------

-- One row per correspondent, shared by every header that names them.
CREATE TABLE addresses (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The addr-spec as first seen, for display.
    address             TEXT    NOT NULL,
    -- Lowercased (`EmailAddress::normalized`), and the identity of the row.
    address_normalized  TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_addresses_normalized ON addresses (address_normalized);

CREATE TABLE recipients (
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

CREATE INDEX idx_recipients_message ON recipients (message_id, kind, position);
CREATE INDEX idx_recipients_draft ON recipients (draft_id, kind, position)
    WHERE draft_id IS NOT NULL;
CREATE INDEX idx_recipients_address ON recipients (address_id, kind);

------------------------------------------------------------------------------
-- Contacts
------------------------------------------------------------------------------

CREATE TABLE contacts (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    -- NULL means the contact is shared across accounts.
    account_id          INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    -- A name the user set, overriding whatever the headers carried.
    name                TEXT,
    address             TEXT    NOT NULL,
    address_name        TEXT,
    address_normalized  TEXT    NOT NULL,
    -- Where this contact came from. `mail` is the passive kind, collected from
    -- headers; the other two the user asked for.
    source              TEXT    NOT NULL DEFAULT 'mail'
                                CHECK (source IN ('mail', 'user', 'import')),
    -- The user has asked never to be offered this address in completion.
    suppressed          INTEGER NOT NULL DEFAULT 0,
    -- vCard identity, and the fields Postio does not model kept verbatim, so a
    -- round trip through Postio does not silently drop them.
    uid                 TEXT,
    vcard_extra         TEXT,
    times_seen          INTEGER NOT NULL DEFAULT 0,
    last_seen_at        INTEGER
);

CREATE UNIQUE INDEX idx_contacts_account_address
    ON contacts (account_id, address_normalized) WHERE account_id IS NOT NULL;
CREATE UNIQUE INDEX idx_contacts_shared_address
    ON contacts (address_normalized) WHERE account_id IS NULL;
CREATE INDEX idx_contacts_rank ON contacts (times_seen DESC, last_seen_at DESC);

CREATE TABLE contact_groups (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    -- NULL means the group is shared across accounts, matching contacts.
    account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    uid        TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE contact_group_members (
    group_id   INTEGER NOT NULL REFERENCES contact_groups(id) ON DELETE CASCADE,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, contact_id)
);

------------------------------------------------------------------------------
-- Settings
------------------------------------------------------------------------------

CREATE TABLE settings (
    key         TEXT    NOT NULL,
    -- NULL scopes the setting globally; otherwise it is per account.
    account_id  INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    -- JSON, so a setting can be richer than a scalar without a migration.
    value       TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_settings_account_key
    ON settings (account_id, key) WHERE account_id IS NOT NULL;
CREATE UNIQUE INDEX idx_settings_global_key
    ON settings (key) WHERE account_id IS NULL;

------------------------------------------------------------------------------
-- The operation queue
------------------------------------------------------------------------------

-- Every mutating action is local-first: the row is written, an operation is
-- enqueued, the UI repaints. This table is the queue, and with drafts it is
-- the only part of the store that cannot be refetched from the server.
CREATE TABLE operation_queue (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id           INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    op_type              TEXT    NOT NULL,
    target_kind          TEXT    NOT NULL DEFAULT 'message'
                                 CHECK (target_kind IN ('message', 'thread', 'mailbox',
                                                        'draft', 'account')),
    target_id            INTEGER,
    mailbox_id           INTEGER REFERENCES mailboxes(id) ON DELETE CASCADE,
    -- JSON arguments for the operation.
    payload              TEXT    NOT NULL DEFAULT '{}',
    -- JSON for the operation that undoes this one, so undo reuses this path.
    inverse              TEXT,
    -- The server identity the target had when the operation was enqueued. The
    -- local row may be gone or renumbered by the time this drains.
    source_uid           INTEGER,
    source_uid_validity  INTEGER,
    source_remote_id     TEXT,
    state                TEXT    NOT NULL DEFAULT 'pending'
                                 CHECK (state IN ('pending', 'in_flight', 'done', 'failed')),
    attempts             INTEGER NOT NULL DEFAULT 0,
    last_error           TEXT,
    -- Backoff: the drainer skips rows until this time.
    next_attempt_at      INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE INDEX idx_operation_queue_drain
    ON operation_queue (account_id, state, next_attempt_at, id);
CREATE INDEX idx_operation_queue_target ON operation_queue (target_kind, target_id);

-- A move between two accounts is a saga, not an operation: copy, confirm,
-- delete. Its state has to survive a crash between any two of those.
CREATE TABLE cross_account_moves (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    source_message_id    INTEGER REFERENCES messages(id)  ON DELETE SET NULL,
    source_account_id    INTEGER REFERENCES accounts(id)  ON DELETE SET NULL,
    source_mailbox_id    INTEGER REFERENCES mailboxes(id) ON DELETE SET NULL,
    target_account_id    INTEGER REFERENCES accounts(id)  ON DELETE SET NULL,
    target_mailbox_id    INTEGER REFERENCES mailboxes(id) ON DELETE SET NULL,
    -- The provisional local row in the target account: local-first means the
    -- message appears there immediately, and the saga reconciles.
    target_message_id    INTEGER REFERENCES messages(id)  ON DELETE SET NULL,
    -- The raw RFC 5322 bytes to append, content-addressed. Held here as well
    -- as on the source row because phase 3 deletes that row, and the blob
    -- sweep's reference walk includes this column so the bytes survive the
    -- saga however the race with collection falls.
    raw_blob_id          TEXT,
    -- The Message-ID: phase 1's idempotency key and phase 2's fallback
    -- confirmation, on servers without UIDPLUS.
    rfc_message_id       TEXT,
    phase                TEXT    NOT NULL DEFAULT 'copying'
                                 CHECK (phase IN ('copying', 'unconfirmed', 'confirmed',
                                                  'done', 'aborted')),
    confirmed_uid        INTEGER,
    confirmed_remote_id  TEXT,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE INDEX idx_cross_account_moves_phase ON cross_account_moves (phase);

------------------------------------------------------------------------------
-- The egress log
------------------------------------------------------------------------------

-- Every connection Postio opens, so "nothing leaves this machine that the user
-- did not ask for" is auditable rather than asserted. Hosts and outcomes only:
-- never message content.
CREATE TABLE egress_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at          INTEGER NOT NULL,
    subsystem   TEXT    NOT NULL CHECK (subsystem IN ('imap', 'smtp', 'discovery')),
    -- NULL before an account exists: discovery during onboarding probes
    -- servers for an account not yet created.
    account_id  INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    host        TEXT    NOT NULL,
    port        INTEGER NOT NULL,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('connected', 'failed'))
);

CREATE INDEX idx_egress_log_at ON egress_log (at DESC);

------------------------------------------------------------------------------
-- Mailbox counts
------------------------------------------------------------------------------

-- The sidebar's numbers are maintained here rather than counted, because
-- counting rows in a mailbox of 80,000 messages is not a 16 ms operation.
--
-- A snoozed message counts as snoozed and not as present: it is hidden from
-- the list until its time comes, and a mailbox showing an unread count for
-- mail nobody can see is a mailbox that reads as broken.

CREATE TRIGGER messages_count_insert AFTER INSERT ON messages
WHEN NEW.deleted_locally = 0
BEGIN
    UPDATE mailboxes
       SET total_count = total_count +
               (NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000)),
           unread_count = unread_count +
               ((NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))
                AND NEW.seen = 0),
           flagged_count = flagged_count +
               ((NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))
                AND NEW.flagged = 1),
           snoozed_count = snoozed_count +
               (NEW.snoozed_until IS NOT NULL AND NEW.snoozed_until > (strftime('%s','now') * 1000))
     WHERE id = NEW.mailbox_id;
END;

CREATE TRIGGER messages_count_delete AFTER DELETE ON messages
WHEN OLD.deleted_locally = 0
BEGIN
    -- Clamped at zero: a count that has drifted should degrade to a wrong
    -- number, never to a negative one that reads as a mailbox of length
    -- 4294967295 once it crosses the crate boundary as a u32.
    UPDATE mailboxes
       SET total_count = max(total_count -
               (OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000)), 0),
           unread_count = max(unread_count -
               ((OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))
                AND OLD.seen = 0), 0),
           flagged_count = max(flagged_count -
               ((OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))
                AND OLD.flagged = 1), 0),
           snoozed_count = max(snoozed_count -
               (OLD.snoozed_until IS NOT NULL AND OLD.snoozed_until > (strftime('%s','now') * 1000)), 0)
     WHERE id = OLD.mailbox_id;
END;

CREATE TRIGGER messages_count_update
AFTER UPDATE OF mailbox_id, seen, flagged, deleted_locally, snoozed_until ON messages
BEGIN
    UPDATE mailboxes
       SET total_count = max(total_count - (OLD.deleted_locally = 0 AND
               (OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))), 0),
           unread_count = max(unread_count - (OLD.deleted_locally = 0 AND
               (OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))
               AND OLD.seen = 0), 0),
           flagged_count = max(flagged_count - (OLD.deleted_locally = 0 AND
               (OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))
               AND OLD.flagged = 1), 0),
           snoozed_count = max(snoozed_count - (OLD.deleted_locally = 0 AND
               OLD.snoozed_until IS NOT NULL AND OLD.snoozed_until > (strftime('%s','now') * 1000)), 0)
     WHERE id = OLD.mailbox_id;
    UPDATE mailboxes
       SET total_count = total_count + (NEW.deleted_locally = 0 AND
               (NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))),
           unread_count = unread_count + (NEW.deleted_locally = 0 AND
               (NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))
               AND NEW.seen = 0),
           flagged_count = flagged_count + (NEW.deleted_locally = 0 AND
               (NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))
               AND NEW.flagged = 1),
           snoozed_count = snoozed_count + (NEW.deleted_locally = 0 AND
               NEW.snoozed_until IS NOT NULL AND NEW.snoozed_until > (strftime('%s','now') * 1000))
     WHERE id = NEW.mailbox_id;
END;
