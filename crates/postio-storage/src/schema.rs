//! The schema, at head, as one constant -- and the steps that bring an
//! earlier build's store up to it.
//!
//! # A new store, and an old one
//!
//! [`HEAD`] is the whole schema, and a new store is written from it in one
//! batch. A store an earlier build wrote is stamped with what *that* build's
//! `HEAD` hashed to ([`FINGERPRINT`]), and [`MIGRATIONS`] is how a stamp this
//! build recognises is carried forward in place: snoozes, reminders, drafts,
//! the operation queue and everything else that exists only in this file
//! survive it. A stamp no migration leads from is refused
//! (`Error::SchemaFromAnotherBuild`), and the way forward from there is a
//! fresh store -- `postio_session::start_over` -- rather than a retry, which
//! would meet the same file.
//!
//! # Changing the schema
//!
//! Edit [`HEAD`], and in the same commit:
//!
//! 1. copy the `HEAD` you are replacing, verbatim, to
//!    `tests/schemas/<its fingerprint as eight hex digits>.sql`;
//! 2. append a [`Migration`] from that fingerprint to the new one, whose
//!    statements turn a store at the old schema into one at the new.
//!
//! `schema::tests::a_schema_change_comes_with_the_migration_that_reaches_it`
//! fails until the second is done, and the storage suite's `migrations`
//! case proves every recorded schema migrates to a store shaped exactly as a
//! fresh one. Forward only, no down migration, and a step is never edited
//! once a store may have run it.
//!
//! A store's steps and its new stamp run in one transaction, so a store cut
//! off part-way is left at its old schema and stamp and runs them again on
//! its next open. A step may therefore add and drop columns and copy rows,
//! none of which can be guarded to run twice.
//!
//! A change no statement can express -- a column whose meaning changed, a
//! constraint tightened under rows that break it -- does not get a step. It
//! leaves the earlier stamp unreachable, and that store starts over. CLAUDE.md
//! allows that ("no backwards compatibility") and still asks for the
//! migration wherever one can be written; `docs/notes/2026-10-01-store-
//! migrations-and-starting-over.md` has the reasoning.
//!
//! # What this is not the same as
//!
//! Four things differ from the schema the old engine held, and each is forced
//! rather than chosen:
//!
//! 1. **`body_dictionaries` is gone.** Bodies were plain `TEXT` for a while,
//!    because an index cannot tokenise compressed bytes and the body index
//!    sat on the body column; once it moved to its own folded table (point
//!    2), the column was free to be small again, and `crate::body_codec`
//!    packs it per row — zstd when that is smaller, no shared dictionary.
//! 2. **`body_search` is a sibling table** (`message_search_bodies`), not a
//!    column: the body folded for search. The engine's
//!    tokenizer does not remove diacritics and offers no option to, so the
//!    fold FTS5 did inside its index is done by `postio_model::fold` before
//!    the write.
//! 3. **No table is `WITHOUT ROWID`.** Four were. Turso puts that behind an
//!    experimental flag and will not build a secondary index on such a table,
//!    which `idx_message_labels_label` and `idx_thread_links_thread` need. The
//!    cost is one rowid per row on four narrow tables; the alternative was
//!    losing two indexes that queries depend on.
//! 4. **The two FTS5 virtual tables are not here.** They are indexes now, and
//!    they live with the rest of the search schema in `postio-index`.
//!
//! Everything else is the schema the SQLCipher store held, transcribed by
//! applying its twenty migrations and dumping the result rather than by
//! retyping it. Those migrations went with that engine: a SQLCipher store
//! cannot be read by Turso at all, so there was nothing for them to carry.

/// What this schema hashes to, for `PRAGMA user_version`.
///
/// # Why a hash rather than a number someone maintains
///
/// A hand-kept version integer has to be remembered, and the failure it
/// guards against is exactly the one where somebody did not: a column was
/// added to [`HEAD`] and nothing else changed, so an older store went on
/// opening and failing one statement at a time. Hashing the schema text
/// cannot be forgotten — edit `HEAD` at all and the stamp moves with it.
///
/// It is deliberately *not* a version. Nothing is ordered, nothing is
/// comparable, and there is no "newer": two builds either agree or they do
/// not. What orders them is [`MIGRATIONS`], which names each stamp it starts
/// from and the one it leaves.
///
/// FNV-1a, 32 bits, which is what `user_version` has room for. A collision
/// would let a mismatched store through — the failure this started from
/// rather than a new one — and 32 bits against the handful of schemas a
/// single-user alpha sees is not worth a hashing dependency.
pub const FINGERPRINT: i64 = fingerprint(HEAD);

/// FNV-1a over the schema text, at compile time: what a store written from
/// `schema` is stamped with.
///
/// Folded through `i32` because that is what `user_version` is: a signed
/// 32-bit field. Hashing to `u32` and widening instead makes every hash above
/// `i32::MAX` read back negative, so the stamp never equals itself and every
/// store demands a resync on its second open.
pub const fn fingerprint(schema: &str) -> i64 {
    let bytes = schema.as_bytes();
    let mut hash: u32 = 0x811c_9dc5;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        index += 1;
    }
    hash as i32 as i64
}

/// One step from a schema an earlier build stamped toward this one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// The stamp of the schema this step starts from: that build's
    /// [`FINGERPRINT`].
    pub from: i64,
    /// The stamp of the schema it leaves.
    pub to: i64,
    /// The statements, run in one transaction with the stamp (see the
    /// module docs).
    pub statements: &'static str,
}

/// A stamp written as the eight hex digits `tests/schemas/` names it by.
const fn stamp(hex: u32) -> i64 {
    hex as i32 as i64
}

/// Every step this build can take, oldest first. The last one's `to` is
/// [`FINGERPRINT`]; a test holds it there.
pub const MIGRATIONS: &[Migration] = &[
    // `feature/postio-focus` at 355ac0cd, before its rebase onto `main`
    // brought the three indexes below: the backfill's top-up and the
    // reparse sweep seeking by `sort_at`, and the queue read by state.
    Migration {
        from: stamp(0x3f95_ddb1),
        to: stamp(0xd8c1_e5df),
        statements: "
CREATE INDEX IF NOT EXISTS idx_messages_body_state
    ON messages (mailbox_id, body_state, sort_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_messages_body_problems
    ON messages (mailbox_id, body_encoding_problems, sort_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_operation_queue_state ON operation_queue (state, op_type);
",
    },
    // `feature/postio-focus` at 33641f8e, before `main`'s macOS frontend was
    // merged in: an account's backend location is one column whatever the
    // backend (#1278, the Maildir backend), and a draft records whether it is
    // being written as rich text (#1271).
    //
    // Neither a rename nor a new column can be guarded by `IF NOT EXISTS`;
    // the transaction the steps run in is what keeps a store from meeting
    // them twice.
    Migration {
        from: stamp(0xd8c1_e5df),
        to: stamp(0x8151_85a3),
        statements: "
ALTER TABLE accounts RENAME COLUMN jmap_session_url TO backend_location;

ALTER TABLE drafts ADD COLUMN rich INTEGER NOT NULL DEFAULT 0;
",
    },
    // Content ownership (#1780, ADR 0046): a message's immutable payload --
    // decoded body, header block, MIME parts and their blobs, the search
    // documents -- moves from the mailbox occurrence to an account-scoped
    // `message_contents` row. Every existing message gets content of its
    // own; nothing already stored is taken as evidence that two occurrences
    // share bytes. A backend's native identity establishes sharing on the
    // next resync.
    //
    // It adds and drops columns and copies rows; the transaction the steps
    // run in is what makes that safe to cut off.
    Migration {
        from: stamp(0x8151_85a3),
        to: stamp(0xdcb8_490f),
        statements: concat!(
            include_str!("schema/contents.sql"),
            include_str!("schema/migrate-content.sql"),
            include_str!("schema/content-locations.sql"),
            include_str!("schema/content-projection.sql"),
        ),
    },
];

/// The steps that carry a store stamped `found` to [`FINGERPRINT`], in the
/// order they run: none for a store already there, and `None` when no
/// recorded step leads from `found` -- a schema this build cannot carry
/// forward, whether it is older than the first step or from a build that
/// is not this one's ancestor.
pub fn migrations_from(found: i64) -> Option<Vec<&'static Migration>> {
    let mut at = found;
    let mut path = Vec::new();
    while at != FINGERPRINT {
        // A cycle would be a list that names a stamp twice: never a path.
        if path.len() >= MIGRATIONS.len() {
            return None;
        }
        let step = MIGRATIONS.iter().find(|step| step.from == at)?;
        path.push(step);
        at = step.to;
    }
    Some(path)
}

/// Every table, index and trigger the store needs, in one batch.
///
/// Creation order is tables, then indexes, then triggers, and tables are in
/// alphabetical order rather than dependency order — a foreign key may be
/// declared before the table it names, which is why [`crate::store`] runs this
/// with foreign keys off and turns them on afterwards.
pub const HEAD: &str = concat!(
    include_str!("schema/contents.sql"),
    r#"
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
    -- Where this backend lives, whatever kind it is (#1278): JMAP's session
    -- resource for `backend = 'jmap'`, the tree on disk for
    -- `backend = 'maildir'`, and NULL for the backends that have neither.
    --
    -- One column rather than one per backend. A maildir account is nothing
    -- but a place -- the tree *is* the account -- and a maildir root stored
    -- in a field called `jmap_session_url` would be a lie every future
    -- reader has to work around.
    backend_location     TEXT,

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
, oauth_refresh_lifetime_days INTEGER, is_default INTEGER NOT NULL DEFAULT 0, max_message_size INTEGER);

CREATE TABLE addresses (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The addr-spec as first seen, for display.
    address             TEXT    NOT NULL,
    -- Lowercased (`EmailAddress::normalized`), and the identity of the row.
    address_normalized  TEXT    NOT NULL
);

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

CREATE TABLE contact_group_members (
    group_id   INTEGER NOT NULL REFERENCES contact_groups(id) ON DELETE CASCADE,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, contact_id)
);

CREATE TABLE contact_groups (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    -- NULL means the group is shared across accounts, matching contacts.
    account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    uid        TEXT,
    created_at INTEGER NOT NULL
);

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

-- Who the person has written to (spec 007): one row per address they have
-- sent mail to, in To, Cc or Bcc, maintained at local send and when a Sent
-- folder syncs. The filter's "you wrote to them" guard is one lookup here
-- (FR-111), and recipient completion's "wrote N times" (FR-052). The
-- sender's own addresses are never counted. Recomputed by a resync.
CREATE TABLE correspondents (
    address_id    INTEGER PRIMARY KEY REFERENCES addresses(id),
    -- Messages sent with this address in To, Cc or Bcc, each counted once.
    -- A send that failed takes its one back, so zero is possible, and means
    -- nobody was written to.
    sent_count    INTEGER NOT NULL,
    -- When the latest of them was sent.
    last_sent_at  INTEGER
);

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

-- One delivery of a Focus digest (spec 007): what a `[[focus.digests]]`
-- rule held until it came due, surfaced as one row of Focus's inbox.
-- Lost on a resync like a snooze; the rules themselves live in config.toml.
CREATE TABLE digest_deliveries (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The rule's `name` in `[[focus.digests]]`.
    rule                TEXT    NOT NULL,
    -- When it came due, and when Focus delivered it: later, if Focus was
    -- not running at the due time.
    due_at              INTEGER NOT NULL,
    delivered_at        INTEGER NOT NULL,
    -- Set by `⇧A`: the delivery's row leaves the inbox, and its holds no
    -- longer keep its mail out of it.
    archived_at         INTEGER,
    -- Milestone 2: the summary's statements and references as JSON, and
    -- when they were written; NULL until then.
    summary             TEXT,
    summary_written_at  INTEGER
);

-- Mail a digest rule holds back from Focus's inbox (spec 007, R13). While
-- its delivery is NULL or not archived, Focus's inbox leaves the message
-- out (`focus_excludes`); a release deletes the row and the message
-- rejoins. Search still finds held mail.
CREATE TABLE digest_holds (
    message_id   INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    rule         TEXT    NOT NULL,
    held_at      INTEGER NOT NULL,
    -- NULL while the rule's due time has not come.
    delivery_id  INTEGER REFERENCES digest_deliveries(id) ON DELETE SET NULL
);

CREATE TABLE "drafts" (
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
    -- The Markdown typed in the terminal composer, exactly; NULL when a
    -- frontend that does not write Markdown saved the draft last.
    body_markdown           TEXT,

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
    rfc_message_id          TEXT,

    -- The message a forward was made from (#1686): where a carried
    -- attachment whose bytes were never downloaded is fetched from at send
    -- time. Not `in_reply_to_message_id`, which decides threading headers a
    -- forward must not carry.
    forwarded_message_id    INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    -- The labels chosen for the message (spec 007 US3), space-separated
    -- label ids: applied to its conversation when it is sent. A list column
    -- for `reference_ids`' reason -- only ever read and written whole, with
    -- the row -- and an id whose label has since been deleted is skipped
    -- when it is applied.
    label_ids               TEXT    NOT NULL DEFAULT '',
    -- The iCalendar answer an invitation's reply carries (spec 007 FR-102):
    -- a METHOD:REPLY object, sent as the text/calendar alternative beside
    -- the words. NULL for every draft a person writes.
    calendar_reply          TEXT,
    -- "Remind if no reply" (spec 007 US3, FR-044), in milliseconds: when
    -- the message is sent, its conversation gets a reminder due then. NULL
    -- when the draft asks for none.
    remind_at               INTEGER,
    -- Whether this draft is being written as rich text (#1271). Last, where
    -- the step from d8c1e5df's `ALTER TABLE ... ADD COLUMN` puts it, so a
    -- migrated store and a fresh one are shaped alike.
    --
    -- Not derived from `body_html`. `rich` used to mean "has an HTML part",
    -- which reads correctly right up to the moment somebody turns the switch
    -- off: the switch is on the *document*, so turning it off changes what
    -- will be built and must not throw the marks away in case it is turned
    -- back on. A derived flag cannot express "has marks, sending plain".
    rich                    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE egress_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at          INTEGER NOT NULL,
    -- 'model' is the person's own model runtime, on this computer (spec 007
    -- FR-168): every call Focus makes to it, by where and outcome.
    subsystem   TEXT    NOT NULL CHECK (subsystem IN ('imap', 'smtp', 'discovery', 'model')),
    -- NULL before an account exists: discovery during onboarding probes
    -- servers for an account not yet created.
    account_id  INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    host        TEXT    NOT NULL,
    port        INTEGER NOT NULL,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('connected', 'failed'))
);

-- Why Focus filed a message out of the inbox (spec 007, FR-113): one
-- decision per message, from a vocabulary this CHECK keeps closed, and the
-- same six `postio_classify::ReasonKind` spells. Recomputable from the
-- message by the catch-up. Restoring it (`R`) keeps the row and marks it
-- `restored_at`: SC-012 measures filtering from the decisions and the
-- restores the store keeps, and undoing a restore puts back the reason it
-- had.
CREATE TABLE filter_decisions (
    message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    reason      TEXT    NOT NULL
                        CHECK (reason IN ('spam', 'promotion', 'notification',
                                          'receipt', 'shipping', 'social')),
    -- Shown after the reason, "notification · Forge": a sender's or a
    -- list's name, or NULL when there is none to show.
    source      TEXT,
    -- Which layer decided: the message's headers, the shipped senders
    -- table, the server's `$Junk`, or the person's model (milestone 2).
    layer       TEXT    NOT NULL
                        CHECK (layer IN ('header', 'senders', 'server', 'model')),
    -- When. Local midnight bounds "filtered today".
    decided_at  INTEGER NOT NULL,
    -- When the person restored the message to the inbox (`R`, FR-116): it
    -- is no longer filtered, and a read of what is filtered leaves it out.
    restored_at INTEGER
);

-- What Focus has classified (spec 007): one record per message and stage,
-- at the version of the classifier that did it. The catch-up at Focus's
-- start reads the rows with none at the current version, newest first
-- (FR-141), so a newer classifier makes every one of them due again.
-- Recomputed by a resync, like the markers and decisions it leads to.
CREATE TABLE focus_classified (
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    -- 'filing': as the mail was filed, or caught up for mail another app
    -- filed; 'body': the body stage, once the body is here.
    stage       TEXT    NOT NULL CHECK (stage IN ('filing', 'body')),
    version     INTEGER NOT NULL,
    PRIMARY KEY (message_id, stage)
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

CREATE TABLE labels (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    -- Optional hex colour, e.g. `#5980a6`.
    color       TEXT
);

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
);

CREATE TABLE mailbox_roles (
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    role        TEXT    NOT NULL
                        CHECK (role IN ('archive', 'sent', 'drafts', 'trash', 'junk')),
    path        TEXT    NOT NULL CHECK (length(path) > 0),
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (account_id, role)
);

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
    -- Visible messages whose body is not here yet (`body_state` not_fetched
    -- or headers_only). What a search asks to say whether its corpus is
    -- complete: `body_state` is in no index, so the question was a walk of
    -- every message once backfill had finished (#1612). Snoozing does not
    -- matter to it: a snoozed message is still searched.
    bodies_owed        INTEGER NOT NULL DEFAULT 0,
    -- A mailbox the user has told the backfill to leave alone (ADR 0016).
    backfill_excluded  INTEGER NOT NULL DEFAULT 0,
    signature_id       INTEGER REFERENCES signatures(id) ON DELETE SET NULL,
    last_synced_at     INTEGER
);

-- Focus's markers (spec 007): what a conversation's row calls out, at most
-- one per message. Written by Focus's body task; read with a Focus page in
-- one batched statement, so drawing a row reads no body. Recomputable from
-- the message, except a dismissal, which a resync forgets like a snooze.
CREATE TABLE markers (
    message_id       INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    -- What the row calls out. `no_reply` is a fired reminder's.
    kind             TEXT    NOT NULL
                             CHECK (kind IN ('invite', 'question', 'todo', 'no_reply')),
    -- What made it: a calendar part, the built-in detector, the person's
    -- own model, or a reminder.
    source           TEXT    NOT NULL
                             CHECK (source IN ('calendar', 'detector', 'model', 'reminder')),
    -- Character offsets of the sentence in the message's own text; NULL for
    -- an invitation or a reminder.
    span_start       INTEGER,
    span_end         INTEGER,
    -- The sentence, verbatim, at most 200 characters; NULL for an invitation.
    excerpt          TEXT,
    -- An invitation's event, UTC milliseconds.
    starts_at        INTEGER,
    ends_at          INTEGER,
    -- A to-do's due date; for `no_reply`, the day the reminder was set.
    due_at           INTEGER,
    -- The invitation's iTIP identity: which later REQUEST or CANCEL
    -- replaces this marker (research R9).
    invite_uid       TEXT,
    invite_sequence  INTEGER,
    invite_stamp     INTEGER,
    invite_state     TEXT    CHECK (invite_state IN ('open', 'cancelled', 'past')),
    -- How the person answered; the `-ing` two last while the reply's window
    -- is open.
    answer           TEXT    CHECK (answer IN ('accepting', 'accepted',
                                               'declining', 'declined')),
    -- While an answer's reply waits out its window in the outbox, when the
    -- window closes (research R9): Focus's due timer then makes the answer
    -- final. NULL otherwise.
    answer_until     INTEGER,
    -- When the person dismissed it. It never returns on this message.
    dismissed_at     INTEGER
);

CREATE TABLE message_labels (
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label_id    INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, label_id)
);

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
    -- When the server received it. Always known, and what the query views,
    -- search and a conversation's own chronology are ordered by.
    received_at             INTEGER NOT NULL,
    -- The message's place in a folder's lists (spec 007, research R7):
    -- `received_at` when it is filed, and the wake time when a snooze wakes,
    -- so a woken message comes back at the top. The folder and conversation
    -- lists, their seek marks and their indexes order by it. A write that
    -- moves `received_at` later -- a draft's re-save -- takes it along; an
    -- earlier one leaves it where it is.
    sort_at                 INTEGER NOT NULL,
    -- `List-Id`, for the mailing-list filters.
    list_id                 TEXT,
    -- What `List-Unsubscribe`, `Precedence` and `Auto-Submitted` say (spec
    -- 007, research R8): headers promoted from the body-time header index
    -- (ADR 0025) so Focus's filing pass knows them at arrival. NULL while
    -- not known: an incremental sync asks for them, a first sync does not,
    -- and the body's own headers fill them otherwise. A write that does not
    -- know them never erases what one did.
    --
    -- `unsubscribe_offered`: 0 or 1, whether it carries `List-Unsubscribe`.
    -- `automation`: a bitmask, 1 `Precedence: bulk`, 2 `list`, 4 `junk`,
    -- 8 `Auto-Submitted: auto-generated` (or any value but `no` and
    -- `auto-replied`), 16 `auto-replied`. `is:bulk` answers from the first
    -- and bits 1-4, `is:automated` from bits 8 and 16.
    unsubscribe_offered     INTEGER,
    automation              INTEGER,
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
    -- Whether the sender asked to be told the message was opened. Postio never
    -- answers one automatically (PRODUCT.md §9); this column is what the
    -- reader shows a notice for.
    read_receipt_requested  INTEGER NOT NULL DEFAULT 0,

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

    -- Outbound state, for a message this client is sending rather than one it
    -- received. NULL for everything that arrived.
    send_state              TEXT
                                    CHECK (send_state IS NULL
                                           OR send_state IN ('editing', 'queued', 'sending',
                                                             'sent', 'failed', 'unconfirmed')),
    send_at                 INTEGER,

    -- Large decoded bodies and the header block live in message_contents.
    -- These small fields are the mailbox list's bounded read projection.
    -- Whether `body_text` uses RFC 3676 format=flowed.
    text_is_flowed          INTEGER NOT NULL DEFAULT 0,
    -- Whether `body_headers` was cut at the header-block limit.
    body_headers_truncated  INTEGER NOT NULL DEFAULT 0,
    -- Whether decoding the body hit a charset or transfer-encoding problem the
    -- reader should disclose rather than hide.
    body_encoding_problems  INTEGER NOT NULL DEFAULT 0,
    -- Which parser wrote `body_text`/`body_html`
    -- (`postio_model::mime::PARSER_VERSION`). A body is fetched once and the
    -- raw bytes are not kept, so a parser fix cannot reach a stored body by
    -- re-parsing it; what it can do is fetch again the rows it got wrong.
    -- A row below the current version that carried the caveat is a backfill
    -- candidate once more (an empty body from a failed decode carries it
    -- too). Zero is "a parser before this column existed".
    body_parsed_with        INTEGER NOT NULL DEFAULT 0,
    -- Lines in `body_text`, for the reader's "show more" threshold, so that
    -- decision never loads the body.
    body_line_count         INTEGER,

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
    html_part_headers       TEXT,
    -- The immutable payload this occurrence shows (ADR 0046): its decoded
    -- body, header block and MIME parts live on `message_contents`, shared
    -- by every occurrence a backend names as the same message. Appended
    -- last because the content-ownership migration adds them with
    -- `ALTER TABLE`, and a fresh store has to be shaped as a migrated one.
    content_id              INTEGER REFERENCES message_contents(id) ON DELETE CASCADE,
    -- The backend's native identity for that payload (a JMAP blob, a Gmail
    -- message id); NULL for IMAP and Maildir, whose occurrences never share.
    content_namespace       TEXT,
    content_key             TEXT,
    -- Whether the content holds a header block, kept here so the header
    -- backfill can find what still needs one without reading contents.
    body_has_headers        INTEGER NOT NULL DEFAULT 0
);

-- The body folded for the full-text index, one row per indexed message.
--
-- A sibling of `messages` rather than a column on it, and the reason is
-- write cost: the body index is a `USING fts` index, and the engine merges
-- tantivy segments on **any** write to the table the index is on. With the
-- index on `messages` a header sync -- which never touches a body -- paid
-- whatever merge the body backfill had made due, measured at 14.6 ms mean
-- and 529 ms worst per header insert against 2.9 ms / 77 ms without
-- (`examples/fts_write_cost.rs`). Moving `body_search` here leaves writes to
-- `messages` -- header syncs, flag flips, moves -- clear of the body index,
-- which now only merges when a body is actually written. This is the same
-- shape `search_documents` already uses for the metadata index, for a
-- gentler version of the same reason.
--
-- Not the body. `body_text` -- what the reader displays -- stays a row in
-- `messages` (ADR 0020, rows not files, is untouched). This is the derived,
-- folded, never-displayed copy the index reads: the engine's tokenizer
-- lowercases and does not strip diacritics, so `postio_model::fold` folds it
-- on the way in and the query path applies the identical fold, both or
-- neither (`café` and `cafe` stop meeting otherwise).
--
-- A row's *presence* is the record that the message was indexed:
-- `messages_missing_body_text` asks for messages with no row here, and
-- `index_body` writes an empty-string row for an attachment-only message so
-- it is not re-selected for ever (#500). The `messages_body_fts` index over
-- `body_search` is created by `postio-index`, which owns the search indexes;
-- the table is here because it hangs off `messages`.
CREATE TABLE message_search_bodies (
    content_id  INTEGER PRIMARY KEY REFERENCES message_contents(id) ON DELETE CASCADE,
    body_search TEXT NOT NULL
);

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

-- A reminder to follow up (spec 007 US5, research R7): when nobody but the
-- person has written in the conversation by `due_at`, the conversation
-- comes back to the top of Focus's inbox, marked "No reply since" the day
-- the reminder was set. Set and cleared by `remind_if_no_reply`, fired by
-- Focus's due timer, and lost on a resync, like a snooze: the person's
-- intent, but a short-lived one.
CREATE TABLE reminders (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The conversation. Not a foreign key: a threading pass may renumber
    -- conversations, and a message's own `thread_id` is not one either.
    thread_id          INTEGER NOT NULL,
    -- The message it was set on: the one the person waits on a reply to.
    anchor_message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    set_at             INTEGER NOT NULL,
    due_at             INTEGER NOT NULL,
    -- When the due timer found no reply by `due_at`, and surfaced it.
    fired_at           INTEGER,
    -- When a message from somebody else arrived in the conversation before
    -- it fired: a reply cancels it (FR-044).
    cancelled_at       INTEGER,
    -- When a surfaced reminder stopped standing: a reply came after all.
    settled_at         INTEGER
);

CREATE TABLE settings (
    key         TEXT    NOT NULL,
    -- NULL scopes the setting globally; otherwise it is per account.
    account_id  INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    -- JSON, so a setting can be richer than a scalar without a migration.
    value       TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
);

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

CREATE TABLE thread_links (
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    -- The *folded* key (`RfcMessageId::folded`): trimmed, angle-bracketed
    -- and lowercased, so binary equality is the type's own case-insensitive
    -- identity and a plain index serves it (#1587). The wire-facing columns
    -- on `messages` keep original case; this table never reaches the wire.
    rfc_message_id TEXT    NOT NULL,
    thread_id      INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    PRIMARY KEY (account_id, rfc_message_id)
);

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

CREATE TABLE unsubscribe_activations (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    list_identifier TEXT    NOT NULL,
    activated_at    INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_addresses_normalized ON addresses (address_normalized);

CREATE INDEX idx_attachments_blob ON attachments (blob_id);

CREATE INDEX idx_attachments_draft
    ON attachments (draft_id, position);

CREATE INDEX idx_attachments_filename ON attachments (filename);

CREATE INDEX idx_attachments_message ON attachments (message_id, position);

CREATE UNIQUE INDEX idx_contacts_account_address
    ON contacts (account_id, address_normalized) WHERE account_id IS NOT NULL;

CREATE INDEX idx_contacts_rank ON contacts (
    (CASE WHEN source = 'mail' THEN 1 ELSE 0 END),
    last_seen_at DESC,
    times_seen DESC,
    id
);

CREATE UNIQUE INDEX idx_contacts_shared_address
    ON contacts (address_normalized) WHERE account_id IS NULL;

CREATE INDEX idx_cross_account_moves_phase ON cross_account_moves (phase);

-- The deliveries not yet archived: the digest rows of Focus's inbox.
CREATE INDEX idx_digest_deliveries_open ON digest_deliveries (archived_at);

CREATE INDEX idx_digest_holds_delivery ON digest_holds (delivery_id);

CREATE INDEX idx_digest_holds_rule ON digest_holds (rule, delivery_id);

CREATE INDEX idx_drafts_account_updated ON drafts (account_id, updated_at DESC);

CREATE INDEX idx_drafts_message ON drafts (message_id);

CREATE INDEX idx_drafts_state ON drafts (state, updated_at);

CREATE INDEX idx_drafts_thread ON drafts (thread_id);

CREATE INDEX idx_egress_log_at ON egress_log (at DESC);

CREATE INDEX idx_filter_decisions_decided ON filter_decisions (decided_at);

CREATE INDEX idx_filter_decisions_reason
    ON filter_decisions (reason, restored_at, decided_at DESC);

CREATE INDEX idx_filter_decisions_standing
    ON filter_decisions (restored_at, decided_at DESC);

CREATE INDEX idx_identities_account ON identities (account_id, position);

CREATE UNIQUE INDEX idx_identities_one_default
    ON identities (account_id) WHERE is_default = 1;

CREATE UNIQUE INDEX idx_labels_account_name ON labels (account_id, name COLLATE NOCASE);

CREATE UNIQUE INDEX idx_mailboxes_account_path ON mailboxes (account_id, path);

CREATE INDEX idx_mailboxes_account_role ON mailboxes (account_id, role);

CREATE INDEX idx_mailboxes_parent ON mailboxes (parent_id);

-- The answers waiting out their windows, for Focus's due timer.
CREATE INDEX idx_markers_answer_until ON markers (answer_until);

CREATE INDEX idx_markers_invite ON markers (invite_uid);

-- The has-action filter's seek (spec 007 T048): the markers the person has
-- not dismissed, a few hundred where an inbox is a hundred thousand. A full
-- index, because this engine's planner does not read a partial one.
CREATE INDEX idx_markers_dismissed ON markers (dismissed_at, message_id);

CREATE INDEX idx_message_labels_label ON message_labels (label_id, message_id);

CREATE INDEX idx_messages_account_list
    ON messages (account_id, received_at DESC, id DESC, deleted_locally, snoozed_until);

CREATE INDEX idx_messages_in_reply_to
    ON messages (account_id, in_reply_to);

CREATE INDEX idx_messages_list
    ON messages (mailbox_id, sort_at DESC, id DESC, deleted_locally, snoozed_until);

-- The backfill's top-up: a folder's newest messages that still owe a body.
-- Newest first puts the bodies already fetched at the front, so a read that
-- walks the folder and filters walks more of them on each of the hundreds of
-- top-ups a backfill makes. `body_state` leads the order so each state seeks
-- straight to the rows it wants, already in the order they are wanted.
CREATE INDEX idx_messages_body_state
    ON messages (mailbox_id, body_state, sort_at DESC, id DESC);

-- The same question for a body an older parser got wrong (see
-- `body_parsed_with`): nearly every message has `body_encoding_problems = 0`,
-- and the seek lands on the few that do not.
CREATE INDEX idx_messages_body_problems
    ON messages (mailbox_id, body_encoding_problems, sort_at DESC, id DESC);

CREATE INDEX idx_messages_list_id ON messages (account_id, list_id);

CREATE INDEX idx_messages_mailbox_remote_id ON messages (mailbox_id, remote_id);

CREATE INDEX idx_messages_mod_seq ON messages (mailbox_id, mod_seq);

-- A folder's thread count, read from the index alone (#1607, #1610).
CREATE INDEX idx_messages_mailbox_threads
    ON messages (mailbox_id, deleted_locally, snoozed_until, thread_id);

CREATE INDEX idx_messages_recency
    ON messages (received_at DESC, id DESC, deleted_locally, snoozed_until);

CREATE INDEX idx_messages_rfc_message_id
    ON messages (account_id, rfc_message_id);

CREATE INDEX idx_messages_send_state
    ON messages (account_id, send_state, deleted_locally);

CREATE INDEX idx_messages_snoozed_due
    ON messages (account_id, snoozed_until, mailbox_id);

CREATE INDEX idx_messages_thread
    ON messages (thread_id, received_at, id, deleted_locally, snoozed_until);

CREATE INDEX idx_messages_thread_mailbox
    ON messages (thread_id, mailbox_id, sort_at DESC, id DESC);

CREATE UNIQUE INDEX idx_messages_uid
    ON messages (mailbox_id, uid_validity, uid) WHERE uid IS NOT NULL;

CREATE INDEX idx_operation_queue_drain
    ON operation_queue (account_id, state, next_attempt_at, id);

CREATE INDEX idx_operation_queue_target ON operation_queue (target_kind, target_id);

-- What the undrained queue holds, by state. Every sync batch asks "which
-- moves, deletes and flag changes has the user made that the server has not
-- heard about yet" (#368, #317), and the answer is the few rows in
-- 'pending' or 'in_flight' among a table that also keeps every settled row
-- for the undo window. Without this the batch read all of them, per batch.
CREATE INDEX idx_operation_queue_state ON operation_queue (state, op_type);

CREATE INDEX idx_recipients_address ON recipients (address_id, kind);

CREATE INDEX idx_recipients_draft ON recipients (draft_id, kind, position);

CREATE INDEX idx_recipients_message ON recipients (message_id, kind, position);

-- The reminders that still stand, in the order they fired: led by the two
-- columns every read of them asks `IS NULL` of, because the planner seeks
-- an equality and will not seek `fired_at IS NOT NULL` from the front of
-- an index. The due timer's read and the surfaced rows' both seek it.
CREATE INDEX idx_reminders_standing ON reminders (settled_at, cancelled_at, fired_at);

CREATE INDEX idx_reminders_thread ON reminders (thread_id);

CREATE UNIQUE INDEX idx_settings_account_key
    ON settings (account_id, key) WHERE account_id IS NOT NULL;

CREATE UNIQUE INDEX idx_settings_global_key
    ON settings (key) WHERE account_id IS NULL;

CREATE INDEX idx_signatures_account ON signatures (account_id, position);

CREATE UNIQUE INDEX idx_signatures_name ON signatures (account_id, name);

CREATE INDEX idx_sync_state_account ON sync_state (account_id);

-- Plain, not `COLLATE NOCASE` (#1587): Turso's planner will not bind an
-- equality through a collated index column — it seeks to `account_id` and
-- walks every link the account has, once per message of a sync, which is a
-- per-row cost that grows with the store. The case-insensitive identity
-- moved into the key itself: `ThreadingRepository` writes and looks up
-- `RfcMessageId::folded`, so binary equality means what NOCASE meant.
-- The sibling of the rule the `_read` indexes already record: this planner
-- reads through neither partial nor collated indexes.
CREATE UNIQUE INDEX idx_thread_links_lookup
    ON thread_links (account_id, rfc_message_id);

CREATE INDEX idx_thread_links_thread ON thread_links (thread_id);

CREATE INDEX idx_threads_account_last_at ON threads (account_id, last_at DESC, id DESC);

CREATE INDEX idx_threads_account_subject ON threads (account_id, subject);

CREATE INDEX idx_threads_last_at ON threads (last_at DESC, id DESC);

CREATE INDEX idx_threads_subject ON threads (subject);

CREATE INDEX idx_unsubscribe_activations_account
    ON unsubscribe_activations (account_id, activated_at DESC);


-- Read companions for the partial UNIQUE indexes above.
--
-- # Why these exist
--
-- The engine **enforces** a partial unique index correctly -- a duplicate
-- inside the predicate is refused and a row outside it is allowed -- but its
-- planner will not *read* through one: a query that matches a partial index
-- exactly still gets `SCAN`. Verified directly in
-- `turso_capabilities.rs::the_planner_does_not_use_a_partial_index`.
--
-- Every non-unique partial index in this schema simply dropped its predicate,
-- which costs a little index size and nothing else. The six unique ones
-- cannot: the predicate is what makes them mean "one default identity *per
-- account*" rather than "one default identity", and dropping it would change
-- what the schema forbids.
--
-- So the constraint keeps its partial index and the read path gets a
-- non-unique twin. That is one more index to write on each of these tables,
-- which is the price of the planner limitation and is written down here so
-- the day it lifts, these can go.
--
-- `idx_messages_uid` is the one that made this non-optional: `upsert_batch`
-- looks a message up by `(mailbox_id, uid_validity, uid)` once per message,
-- so a scan there is a scan per message of a sync.
CREATE INDEX idx_messages_uid_read ON messages (mailbox_id, uid_validity, uid);

CREATE INDEX idx_contacts_account_address_read
    ON contacts (account_id, address_normalized);

CREATE INDEX idx_contacts_shared_address_read ON contacts (address_normalized);

CREATE INDEX idx_identities_default_read ON identities (account_id, is_default);

CREATE INDEX idx_settings_account_key_read ON settings (account_id, key);

CREATE INDEX idx_settings_global_key_read ON settings (key);

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
               (OLD.snoozed_until IS NOT NULL AND OLD.snoozed_until > (strftime('%s','now') * 1000)), 0),
           bodies_owed = max(bodies_owed - (OLD.body_state IN ('not_fetched', 'headers_only')), 0)
     WHERE id = OLD.mailbox_id;
END;

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
               (NEW.snoozed_until IS NOT NULL AND NEW.snoozed_until > (strftime('%s','now') * 1000)),
           bodies_owed = bodies_owed + (NEW.body_state IN ('not_fetched', 'headers_only'))
     WHERE id = NEW.mailbox_id;
END;

-- `bodies_owed` has a trigger of its own rather than joining the one below:
-- a body arriving is the backfill's every write, and the four counts below
-- would be rewritten for nothing each time. This one fires only when the
-- owed answer or the folder actually changed.
CREATE TRIGGER messages_bodies_owed_update
AFTER UPDATE OF mailbox_id, deleted_locally, body_state ON messages
WHEN OLD.mailbox_id <> NEW.mailbox_id
  OR (OLD.deleted_locally = 0 AND OLD.body_state IN ('not_fetched', 'headers_only'))
  <> (NEW.deleted_locally = 0 AND NEW.body_state IN ('not_fetched', 'headers_only'))
BEGIN
    UPDATE mailboxes
       SET bodies_owed = max(bodies_owed - (OLD.deleted_locally = 0
               AND OLD.body_state IN ('not_fetched', 'headers_only')), 0)
     WHERE id = OLD.mailbox_id;
    UPDATE mailboxes
       SET bodies_owed = bodies_owed + (NEW.deleted_locally = 0
               AND NEW.body_state IN ('not_fetched', 'headers_only'))
     WHERE id = NEW.mailbox_id;
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
END;;"#,
    include_str!("schema/content-locations.sql"),
    include_str!("schema/content-projection.sql")
);

/// What the schema declares, as names, without an engine.
///
/// A crude parse on purpose: it reads [`HEAD`] the way a reader does rather
/// than the way an engine does, so the test below can run at the `--lib` tier
/// in microseconds instead of opening a database.
#[cfg(test)]
fn declared() -> std::collections::BTreeSet<&'static str> {
    HEAD.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line
                .strip_prefix("CREATE TABLE ")
                .or_else(|| line.strip_prefix("CREATE INDEX "))
                .or_else(|| line.strip_prefix("CREATE UNIQUE INDEX "))
                .or_else(|| line.strip_prefix("CREATE TRIGGER "))?;
            rest.trim_matches('"').split([' ', '(', '"']).next()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every object the twenty migrations built, minus the ones this engine
    /// deliberately does without.
    ///
    /// Generated by applying `src/migrations/*.sql` in order and reading
    /// `sqlite_master`, so it describes the schema as it actually was rather
    /// than as anyone remembers it. Transcription is the risk this test
    /// exists for: `HEAD` was assembled from a dump, and a dump that lost an
    /// index would still be a working schema and a slow one.
    const OLD_SCHEMA: &[&str] = &[
        // 26 tables
        "accounts",
        "addresses",
        "attachments",
        "body_dictionaries",
        "contact_group_members",
        "contact_groups",
        "contacts",
        "cross_account_moves",
        "drafts",
        "egress_log",
        "identities",
        "labels",
        "mailbox_role_refusals",
        "mailbox_roles",
        "mailboxes",
        "message_labels",
        "messages",
        "operation_queue",
        "recipients",
        "settings",
        "signatures",
        "sqlite_sequence",
        "sync_state",
        "thread_links",
        "threads",
        "unsubscribe_activations",
        // 55 indexes
        "idx_addresses_normalized",
        "idx_attachments_blob",
        "idx_attachments_draft",
        "idx_attachments_filename",
        "idx_attachments_message",
        "idx_contacts_account_address",
        "idx_contacts_rank",
        "idx_contacts_shared_address",
        "idx_cross_account_moves_phase",
        "idx_drafts_account_updated",
        "idx_drafts_message",
        "idx_drafts_state",
        "idx_drafts_thread",
        "idx_egress_log_at",
        "idx_identities_account",
        "idx_identities_one_default",
        "idx_labels_account_name",
        "idx_mailboxes_account_path",
        "idx_mailboxes_account_role",
        "idx_mailboxes_parent",
        "idx_message_labels_label",
        "idx_messages_account_list",
        "idx_messages_in_reply_to",
        "idx_messages_list",
        "idx_messages_list_id",
        "idx_messages_mailbox_remote_id",
        "idx_messages_mod_seq",
        "idx_messages_recency",
        "idx_messages_rfc_message_id",
        "idx_messages_send_state",
        "idx_messages_snoozed_due",
        "idx_messages_thread",
        "idx_messages_thread_mailbox",
        "idx_messages_uid",
        "idx_operation_queue_drain",
        "idx_operation_queue_target",
        "idx_recipients_address",
        "idx_recipients_draft",
        "idx_recipients_message",
        "idx_settings_account_key",
        "idx_settings_global_key",
        "idx_signatures_account",
        "idx_signatures_name",
        "idx_sync_state_account",
        "idx_thread_links_lookup",
        "idx_thread_links_thread",
        "idx_threads_account_last_at",
        "idx_threads_account_subject",
        "idx_threads_last_at",
        "idx_threads_subject",
        "idx_unsubscribe_activations_account",
        // 4 triggers
        "messages_bodies_owed_update",
        "messages_count_delete",
        "messages_count_insert",
        "messages_count_update",
    ];

    /// Gone on purpose, each with the reason it is gone.
    ///
    /// The list that makes this test a record rather than a rubber stamp: an
    /// object may only leave the schema by being named here, so "we dropped
    /// it deliberately" has to be written down at the moment it stops being
    /// true that nothing was lost.
    const DELIBERATELY_ABSENT: &[(&str, &str)] = &[
        (
            "body_dictionaries",
            "the zstd dictionaries the bodies were compressed against. Bodies \
             are TEXT now because the full-text index is built on the column \
             itself, so there is nothing left to compress against.",
        ),
        (
            "sqlite_sequence",
            "the engine's own bookkeeping for AUTOINCREMENT, never declared \
             by a migration -- it appeared in the dump because the engine \
             creates it. Turso creates its own.",
        ),
    ];

    #[test]
    fn the_head_schema_declares_everything_the_migrations_did() {
        let declared = declared();
        let excused: std::collections::BTreeSet<&str> =
            DELIBERATELY_ABSENT.iter().map(|(name, _)| *name).collect();

        let missing: Vec<&str> = OLD_SCHEMA
            .iter()
            .copied()
            .filter(|name| !declared.contains(name) && !excused.contains(name))
            .collect();

        assert!(
            missing.is_empty(),
            "the head schema lost {} object(s) the migrations declared: {missing:?}\n\
             Either transcribe them into HEAD, or name each one in \
             DELIBERATELY_ABSENT with the reason it is gone.",
            missing.len(),
        );
    }

    #[test]
    fn nothing_is_excused_that_the_schema_still_declares() {
        let declared = declared();
        let contradictory: Vec<&str> = DELIBERATELY_ABSENT
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| declared.contains(name))
            .collect();
        assert!(
            contradictory.is_empty(),
            "DELIBERATELY_ABSENT claims {contradictory:?} were dropped, but \
             HEAD still declares them -- the reasons recorded there are stale.",
        );
    }

    /// A schema change without its migration is caught here, before any
    /// store meets it: editing `HEAD` moves [`FINGERPRINT`], and until a
    /// step names the new stamp as its `to`, every store the previous build
    /// wrote would be refused and have to start over.
    #[test]
    fn a_schema_change_comes_with_the_migration_that_reaches_it() {
        let last = MIGRATIONS.last().expect("at least one migration");
        assert_eq!(
            last.to,
            FINGERPRINT,
            "HEAD now hashes to {:08x}, and no migration leads there. Copy the \
             HEAD you replaced to tests/schemas/{:08x}.sql and append a \
             Migration from {:08x} to {:08x} whose statements make a store at \
             the old schema one at the new",
            FINGERPRINT as i32 as u32,
            last.to as i32 as u32,
            last.to as i32 as u32,
            FINGERPRINT as i32 as u32,
        );
    }

    #[test]
    fn every_recorded_stamp_reaches_head() {
        for step in MIGRATIONS {
            assert!(
                migrations_from(step.from).is_some(),
                "a store stamped {:08x} has a first step and no way to head",
                step.from as i32 as u32
            );
        }
        assert_eq!(migrations_from(FINGERPRINT), Some(Vec::new()));
    }

    #[test]
    fn a_stamp_nothing_leads_from_has_no_way_to_head() {
        assert_eq!(migrations_from(1), None);
    }
}
