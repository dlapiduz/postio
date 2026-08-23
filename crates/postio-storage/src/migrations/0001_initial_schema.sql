-- Postio's initial schema (spec.md §6).
--
-- Conventions used throughout:
--
--   * Every entity id is `INTEGER PRIMARY KEY AUTOINCREMENT`, matching the
--     `i64` newtypes in `postio_model::ids`. AUTOINCREMENT (rather than a plain
--     rowid) is deliberate: ids are handed to the UI and to the operation
--     queue, and a reused rowid after an expunge would silently retarget a
--     queued operation.
--   * Timestamps are INTEGER milliseconds since the Unix epoch, UTC. Integers
--     so the message-list index sorts without a conversion, milliseconds
--     because `chrono::DateTime<Utc>` round-trips through them exactly at the
--     precision IMAP gives us.
--   * Booleans are INTEGER 0/1.
--   * Enumerations are stored as the stable lowercase snake_case string the
--     model documents (e.g. `MailboxRole::as_str`), with a CHECK constraint so
--     a typo in a repository is a write error and not a silent data loss.
--   * No BLOB columns anywhere. Message bodies, raw RFC 5322 bytes and
--     attachment payloads live in the content-addressed blob store; SQLite
--     holds only the blob key and the metadata needed to list and search.
--   * No secrets. Passwords and tokens live in the Secret Service keyring.

-- Configured mail accounts. Credentials are NOT here; see the keyring.
CREATE TABLE accounts (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    display_name       TEXT    NOT NULL,
    -- The account's primary address, verbatim, plus the display name it
    -- carried. Together these rebuild an `EmailAddress`.
    address            TEXT    NOT NULL,
    address_name       TEXT,
    incoming_host      TEXT    NOT NULL,
    incoming_port      INTEGER NOT NULL,
    incoming_security  TEXT    NOT NULL DEFAULT 'tls'
                               CHECK (incoming_security IN ('none', 'starttls', 'tls')),
    incoming_username  TEXT    NOT NULL,
    outgoing_host      TEXT    NOT NULL,
    outgoing_port      INTEGER NOT NULL,
    outgoing_security  TEXT    NOT NULL DEFAULT 'starttls'
                               CHECK (outgoing_security IN ('none', 'starttls', 'tls')),
    outgoing_username  TEXT    NOT NULL,
    auth_method        TEXT    NOT NULL DEFAULT 'password'
                               CHECK (auth_method IN ('password', 'app_password',
                                                      'oauth2', 'xoauth2')),
    enabled            INTEGER NOT NULL DEFAULT 1,
    created_at         INTEGER NOT NULL
);

-- Addresses the user can send from within an account.
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

-- `Account::default_identity` must be unambiguous.
CREATE UNIQUE INDEX idx_identities_one_default
    ON identities (account_id) WHERE is_default = 1;

-- Server folders, mirrored locally. Note that a mailbox's UIDVALIDITY /
-- UIDNEXT / HIGHESTMODSEQ are NOT here: they live in `sync_state`, so the sync
-- engine can update them atomically with the message writes they describe
-- without touching the row the sidebar reads.
CREATE TABLE mailboxes (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    parent_id      INTEGER REFERENCES mailboxes(id) ON DELETE SET NULL,
    name           TEXT    NOT NULL,
    path           TEXT    NOT NULL,
    -- The server's hierarchy delimiter, a single character, or NULL for a flat
    -- namespace.
    delimiter      TEXT    CHECK (delimiter IS NULL OR length(delimiter) = 1),
    role           TEXT    NOT NULL DEFAULT 'regular'
                           CHECK (role IN ('inbox', 'archive', 'sent', 'drafts',
                                           'trash', 'junk', 'flagged', 'regular')),
    selectable     INTEGER NOT NULL DEFAULT 1,
    subscribed     INTEGER NOT NULL DEFAULT 1,
    -- Cached counts, so the sidebar never counts rows.
    total_count    INTEGER NOT NULL DEFAULT 0,
    unread_count   INTEGER NOT NULL DEFAULT 0,
    flagged_count  INTEGER NOT NULL DEFAULT 0,
    last_synced_at INTEGER
);

CREATE UNIQUE INDEX idx_mailboxes_account_path ON mailboxes (account_id, path);
CREATE INDEX idx_mailboxes_account_role ON mailboxes (account_id, role);
CREATE INDEX idx_mailboxes_parent ON mailboxes (parent_id);

-- Per-mailbox synchronization state, treated as one unit.
--
-- A mailbox with no row here, or with a NULL `uid_validity`, has never been
-- synced — that state is explicit rather than inferred from zero counts.
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

-- Conversations, reconstructed locally by the JWZ pass.
--
-- The aggregate columns are a cache over the thread's messages. Membership,
-- participants, mailboxes and labels are NOT duplicated here: they are derived
-- from `messages.thread_id`, `recipients` and `message_labels`.
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

-- The threaded message list: newest conversation first, no sort step.
CREATE INDEX idx_threads_account_last_at ON threads (account_id, last_at DESC, id DESC);
-- Subject fallback matching during threading.
CREATE INDEX idx_threads_account_subject ON threads (account_id, subject);

-- Messages: the centre of the model.
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
    reference_ids           TEXT NOT NULL DEFAULT '',

    subject                 TEXT,
    -- `Subject` with Re:/Fwd: prefixes stripped, for subject-based threading.
    normalized_subject      TEXT,
    -- The `Date` header, as claimed by the sender; may be absent or a lie.
    date                    INTEGER,
    -- When the server received it. Always known; this is the list sort key.
    received_at             INTEGER NOT NULL,

    -- A short plain-text snippet for the list. The body itself is not here.
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

    -- Content-addressed blob store keys. NULL until the bytes are downloaded.
    raw_blob_id             TEXT,
    body_text_blob_id       TEXT,
    body_html_blob_id       TEXT,
    -- The full header block, preserved for display and later reparsing.
    headers_blob_id         TEXT
);

-- The message list. Windowed paging depends on this index answering
-- `WHERE mailbox_id = ? ORDER BY received_at DESC, id DESC LIMIT ?` with no
-- temp b-tree; see the <16ms interaction budget in CLAUDE.md.
CREATE INDEX idx_messages_list ON messages (mailbox_id, received_at DESC, id DESC);
-- The same list scoped to a whole account (unified views).
CREATE INDEX idx_messages_account_list ON messages (account_id, received_at DESC, id DESC);
-- Thread drill-in: members oldest first, which is `Thread::message_ids` order.
CREATE INDEX idx_messages_thread ON messages (thread_id, received_at, id);
-- JWZ threading resolves parents by Message-ID, once per incoming message.
CREATE INDEX idx_messages_rfc_message_id
    ON messages (account_id, rfc_message_id) WHERE rfc_message_id IS NOT NULL;
CREATE INDEX idx_messages_in_reply_to
    ON messages (account_id, in_reply_to) WHERE in_reply_to IS NOT NULL;
-- The sidebar's "Flagged" view.
CREATE INDEX idx_messages_flagged
    ON messages (account_id, received_at DESC, id DESC) WHERE flagged = 1;
-- QRESYNC: everything changed above a known MODSEQ.
CREATE INDEX idx_messages_mod_seq ON messages (mailbox_id, mod_seq);
-- Sync reconciliation looks a message up by its server identity. Partial so
-- locally composed messages, which have no UID, are unconstrained and may be
-- many.
CREATE UNIQUE INDEX idx_messages_uid
    ON messages (mailbox_id, uid_validity, uid) WHERE uid IS NOT NULL;
-- The backlog the body backfill drains.
CREATE INDEX idx_messages_body_state
    ON messages (mailbox_id, received_at DESC) WHERE body_state <> 'full';

-- Address headers, for messages and for drafts alike.
--
-- One table rather than six columns of packed text: `from:`/`to:` search
-- operators (spec.md §7) index `address_normalized` directly, and `position`
-- preserves header order exactly.
CREATE TABLE recipients (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id          INTEGER REFERENCES messages(id) ON DELETE CASCADE,
    draft_id            INTEGER REFERENCES drafts(id) ON DELETE CASCADE,
    kind                TEXT    NOT NULL
                                CHECK (kind IN ('from', 'sender', 'reply_to',
                                                'to', 'cc', 'bcc')),
    position            INTEGER NOT NULL DEFAULT 0,
    -- Display name as it appeared, and the addr-spec verbatim.
    name                TEXT,
    address             TEXT    NOT NULL,
    -- Lowercased addr-spec (`EmailAddress::normalized`), for lookup.
    address_normalized  TEXT    NOT NULL,
    CHECK ((message_id IS NOT NULL) <> (draft_id IS NOT NULL))
);

CREATE INDEX idx_recipients_message ON recipients (message_id, kind, position);
CREATE INDEX idx_recipients_draft ON recipients (draft_id, kind, position);
CREATE INDEX idx_recipients_address ON recipients (address_normalized, kind);

-- Attachment and inline-part metadata. The bytes are in the blob store.
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
    -- Blob store key; NULL until the bytes have been downloaded.
    blob_id           TEXT,
    CHECK ((message_id IS NOT NULL) <> (draft_id IS NOT NULL)),
    CHECK (disposition <> 'other' OR disposition_raw IS NOT NULL)
);

CREATE INDEX idx_attachments_message ON attachments (message_id, position);
CREATE INDEX idx_attachments_draft ON attachments (draft_id, position);
-- `filename:` search, and finding the rows that reference a blob before a
-- garbage collection pass removes it.
CREATE INDEX idx_attachments_filename ON attachments (filename) WHERE filename IS NOT NULL;
CREATE INDEX idx_attachments_blob ON attachments (blob_id) WHERE blob_id IS NOT NULL;

-- User labels. Backed by IMAP keywords on servers that have no real labels.
CREATE TABLE labels (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    -- Optional hex colour, e.g. `#5980a6`.
    color       TEXT
);

-- Names are unique per account, case-insensitively, as the model documents.
CREATE UNIQUE INDEX idx_labels_account_name ON labels (account_id, name COLLATE NOCASE);

CREATE TABLE message_labels (
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label_id    INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, label_id)
) WITHOUT ROWID;

CREATE INDEX idx_message_labels_label ON message_labels (label_id, message_id);

-- Messages being composed.
--
-- Draft bodies ARE stored inline rather than in the blob store: a draft is the
-- composer's live buffer, autosaved on every keystroke, and churning a
-- content-addressed store with every edit would be wrong. It moves to the blob
-- store when it becomes a sent message.
CREATE TABLE drafts (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id              INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- NULL means the account's default identity.
    identity_id             INTEGER REFERENCES identities(id) ON DELETE SET NULL,
    kind                    TEXT    NOT NULL DEFAULT 'new'
                                    CHECK (kind IN ('new', 'reply', 'reply_all', 'forward')),
    in_reply_to_message_id  INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    thread_id               INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    subject                 TEXT    NOT NULL DEFAULT '',
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

-- Correspondents accumulated from headers, for recipient autocomplete.
--
-- Beyond spec.md §6's list, but `postio_model::Contact` exists and autocomplete
-- has to rank from somewhere.
CREATE TABLE contacts (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    -- NULL means the contact is shared across accounts.
    account_id          INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    -- A name the user set, overriding whatever the headers carried.
    name                TEXT,
    address             TEXT    NOT NULL,
    address_name        TEXT,
    address_normalized  TEXT    NOT NULL,
    times_seen          INTEGER NOT NULL DEFAULT 0,
    last_seen_at        INTEGER
);

CREATE UNIQUE INDEX idx_contacts_account_address
    ON contacts (account_id, address_normalized) WHERE account_id IS NOT NULL;
CREATE UNIQUE INDEX idx_contacts_shared_address
    ON contacts (address_normalized) WHERE account_id IS NULL;
-- Autocomplete ranks by how often and how recently an address was seen.
CREATE INDEX idx_contacts_rank ON contacts (times_seen DESC, last_seen_at DESC);

-- Persisted application state: pane widths, last selected mailbox, and the
-- like. The user's *configuration* is TOML and belongs to `postio-config`;
-- this is state the app owns.
CREATE TABLE settings (
    key         TEXT    NOT NULL,
    -- NULL scopes the setting globally; otherwise it is per account.
    account_id  INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    -- JSON, so a setting can be richer than a scalar without a migration.
    value       TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Two partial indexes rather than one: SQLite treats NULLs in a UNIQUE index as
-- distinct, so a plain `UNIQUE (account_id, key)` would let global keys repeat.
CREATE UNIQUE INDEX idx_settings_account_key
    ON settings (account_id, key) WHERE account_id IS NOT NULL;
CREATE UNIQUE INDEX idx_settings_global_key
    ON settings (key) WHERE account_id IS NULL;

-- The local-first mutation queue: the mechanism behind offline mode and undo.
--
-- Every mutating action writes SQLite and enqueues here in one transaction, and
-- the UI repaints without waiting for the network. `op_type` is deliberately
-- unconstrained: the sync engine owns that vocabulary.
CREATE TABLE operation_queue (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id       INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    op_type          TEXT    NOT NULL,
    target_kind      TEXT    NOT NULL DEFAULT 'message'
                             CHECK (target_kind IN ('message', 'thread', 'mailbox',
                                                    'draft', 'account')),
    target_id        INTEGER,
    mailbox_id       INTEGER REFERENCES mailboxes(id) ON DELETE CASCADE,
    -- JSON arguments for the operation.
    payload          TEXT    NOT NULL DEFAULT '{}',
    -- JSON for the operation that undoes this one, so undo reuses this path.
    inverse          TEXT,
    state            TEXT    NOT NULL DEFAULT 'pending'
                             CHECK (state IN ('pending', 'in_flight', 'done', 'failed')),
    attempts         INTEGER NOT NULL DEFAULT 0,
    last_error       TEXT,
    -- Backoff: the drainer skips rows until this time.
    next_attempt_at  INTEGER,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

-- The drainer's query. Ordering by `id` is what makes the queue survive a
-- restart in enqueue order, which is why ids never get reused.
CREATE INDEX idx_operation_queue_drain
    ON operation_queue (account_id, state, next_attempt_at, id);
-- "Does this message still have operations in flight?"
CREATE INDEX idx_operation_queue_target ON operation_queue (target_kind, target_id);
