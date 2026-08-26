//! The FTS5 search index: schema, sync triggers, and the rebuild path.
//!
//! # Why an external-content table needs a shadow table
//!
//! FTS5's `content=` mode lets a virtual table borrow its rows from an
//! ordinary table instead of duplicating them, which is exactly what a
//! search index over `messages` wants — no second copy of every subject and
//! body. But the columns this index covers (sender, recipients, subject,
//! body text, attachment filenames) do not live on one row of `messages`:
//! the sender and recipients are in `recipients`, the filenames are in
//! `attachments`, and the body text is not in SQLite at all — it lives in
//! the content-addressed blob store (CLAUDE.md, "No BLOB columns anywhere").
//!
//! So `messages_fts` points at `search_documents`, a table this crate
//! owns: one flattened row per message, kept current by triggers on
//! `messages`, `recipients` and `attachments` for the metadata columns, and
//! by [`index_body`] for the body text, since nothing in SQL can compute
//! that column's value the way a trigger computes the others.
//!
//! `search_documents.message_id` cascades from `messages.id`, so deleting a
//! message deletes its shadow row, which the standard external-content sync
//! triggers below turn into the matching `messages_fts` deletion.
//!
//! # Applying this schema
//!
//! [`ensure_schema`] is idempotent — every statement is `IF NOT EXISTS` — so
//! it is safe to call on every connection this crate is handed, the same way
//! `postio_storage::migrate` is safe to call on every start. It is
//! deliberately not a numbered `postio-storage` migration: this index is
//! `postio-search`'s own concern, layered on top of tables `postio-storage`
//! already created.

use postio_model::MessageBody;
use rusqlite::Connection;

use crate::error::Result;

/// Creates `search_documents`, `messages_fts` and every trigger that keeps
/// them in sync, if they do not already exist.
///
/// Call this once per connection before indexing or searching — on every
/// application start, the same way `postio_storage::migrate` runs on every
/// start. Requires `PRAGMA foreign_keys = ON` (set by
/// `postio_storage::db::configure`) so that deleting a message cascades into
/// `search_documents`.
///
/// # It indexes what is already there
///
/// Creating the triggers is only half of it. They see mail that arrives after
/// them, and this index is retro-fitted onto stores that already hold tens of
/// thousands of messages — so the schema ends with a backfill over `messages`,
/// and running it again is a no-op rather than a second copy. Everything
/// except message *bodies*: those live in the blob store, no trigger and no
/// `SELECT` can reach them, and [`index_body`] is how they arrive.
pub fn ensure_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(SCHEMA)?;
    Ok(())
}

/// Sets (or clears) the indexed body text for a message.
///
/// Body text lives in the blob store, not SQLite, so nothing here can derive
/// it the way the metadata columns are derived by trigger. The caller reads
/// the extracted plain-text body (E2.9's job, not this crate's — raw HTML
/// must never reach this column) and passes it here once it has the bytes.
///
/// A message with no `search_documents` row yet (indexing raced ahead of the
/// message insert) is not an error: the write is simply a no-op, since there
/// is nothing to update. See [`ensure_schema`] for why the row always exists
/// once the message does.
pub fn index_body(connection: &Connection, message_id: i64, body: Option<&str>) -> Result<()> {
    connection.execute(
        "UPDATE search_documents SET body = ?1 WHERE message_id = ?2",
        rusqlite::params![body.unwrap_or(""), message_id],
    )?;
    Ok(())
}

/// Indexes a message's body, whichever form it arrived in.
///
/// The call every producer of a body should make, rather than
/// [`index_body`] with text it extracted itself. "Raw markup must never reach
/// this column" is a rule about the column, so the crate that owns the column
/// is where it is kept: an HTML-only message goes through
/// [`postio_body::parse`] and is indexed as what it *says*, never as its
/// markup — otherwise every such message is a hit for `div`, for `href`, and
/// for the host of every tracking redirect it carries.
///
/// `text/plain` wins when there is one. It is what the sender wrote, the
/// HTML alternative is a rendering of the same words, and indexing both would
/// double the index for no new hits.
///
/// A message with neither form clears the column rather than leaving stale
/// text behind — the same shape as `index_body(.., None)`.
pub fn index_body_of(connection: &Connection, message_id: i64, body: &MessageBody) -> Result<()> {
    index_body(connection, message_id, indexable_text(body).as_deref())
}

/// The plain text that represents `body` in the index, if it has any.
///
/// Separate from [`index_body_of`] so it can be tested without a database,
/// and so the maintenance pass and the sync path provably agree on what a
/// message's indexable text *is*.
pub fn indexable_text(body: &MessageBody) -> Option<String> {
    if let Some(text) = body.text.as_deref().filter(|text| !text.trim().is_empty()) {
        return Some(text.to_owned());
    }
    let html = body.html.as_deref()?;
    // `to_search_text`, not `to_text`: the latter spells a link out as
    // `label <href>` because a quoted reply needs the address, and an index
    // must not — see its own documentation. Both walk `postio_body`'s closed
    // document subset rather than doing a general markup-to-text pass, which
    // is the thing that makes most mail's plain-text part unreadable.
    let text = postio_body::parse(html).to_search_text();
    (!text.trim().is_empty()).then_some(text)
}

/// Message ids whose body is local but whose indexed text is empty, newest
/// first and windowed to `limit`.
///
/// What a store that predates body indexing needs to catch up on, and what
/// any body that missed its write at fetch time — a crash between the commit
/// point and the index write — shows up in afterwards. Empty on a store that
/// is already caught up, which is what makes running it on every start
/// affordable.
///
/// `body_state = 'full'` and not merely "has a blob": the column says whether
/// the bytes are on this machine, and a message the index claims to have read
/// while its body is still on the server would make search answer for a
/// corpus it does not have.
pub fn messages_missing_body_text(connection: &Connection, limit: u32) -> Result<Vec<i64>> {
    let mut statement = connection.prepare(
        "SELECT m.id
           FROM messages m
           JOIN search_documents d ON d.message_id = m.id
          WHERE m.body_state = 'full'
            AND d.body = ''
          ORDER BY m.received_at DESC
          LIMIT ?1",
    )?;
    let rows = statement.query_map([limit], |row| row.get::<_, i64>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Rebuilds `messages_fts` from `search_documents`.
///
/// This is FTS5's own `'rebuild'` command: it discards and regenerates the
/// index's internal b-trees from the content table, without touching
/// `search_documents` itself. Use it after a bulk import that bypassed the
/// triggers (a batch insert with triggers temporarily disabled, for
/// instance), or as a maintenance operation if the index is ever suspected
/// to have drifted from its content.
pub fn rebuild(connection: &Connection) -> Result<()> {
    connection.execute_batch("INSERT INTO messages_fts(messages_fts) VALUES ('rebuild');")?;
    Ok(())
}

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS search_documents (
    message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    sender      TEXT NOT NULL DEFAULT '',
    recipients  TEXT NOT NULL DEFAULT '',
    subject     TEXT NOT NULL DEFAULT '',
    body        TEXT NOT NULL DEFAULT '',
    filenames   TEXT NOT NULL DEFAULT '',
    list_id     TEXT NOT NULL DEFAULT ''
);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    sender, recipients, subject, body, filenames, list_id,
    content = 'search_documents',
    content_rowid = 'message_id',
    tokenize = 'unicode61 remove_diacritics 2'
);

-- messages -> search_documents: subject and list_id, both scalar columns on
-- `messages` itself. Sender/recipients/filenames come from their own
-- tables' triggers below, since a message row can be inserted before or
-- after its recipients and attachments.
CREATE TRIGGER IF NOT EXISTS trg_search_documents_messages_ai
AFTER INSERT ON messages
BEGIN
    INSERT INTO search_documents (message_id, subject, list_id)
    VALUES (new.id, coalesce(new.subject, ''), coalesce(new.list_id, ''))
    ON CONFLICT (message_id) DO UPDATE SET subject = excluded.subject, list_id = excluded.list_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_messages_au
AFTER UPDATE OF subject, list_id ON messages
BEGIN
    UPDATE search_documents SET subject = coalesce(new.subject, ''), list_id = coalesce(new.list_id, '')
    WHERE message_id = new.id;
END;

-- recipients -> search_documents: sender (kind = 'from') and recipients
-- (kind in to/cc/bcc), recomputed from scratch for the affected message(s)
-- on every change. A message has a handful of recipient rows at most, so
-- re-aggregating the lot is cheaper than tracking a per-kind delta.
CREATE TRIGGER IF NOT EXISTS trg_search_documents_recipients_ai
AFTER INSERT ON recipients
WHEN new.message_id IS NOT NULL
BEGIN
    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(name, '') || ' ' || address, ' '), '')
                  FROM recipients WHERE message_id = new.message_id AND kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(name, '') || ' ' || address, ' '), '')
                      FROM recipients WHERE message_id = new.message_id AND kind IN ('to', 'cc', 'bcc'))
    WHERE message_id = new.message_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_recipients_ad
AFTER DELETE ON recipients
WHEN old.message_id IS NOT NULL
BEGIN
    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(name, '') || ' ' || address, ' '), '')
                  FROM recipients WHERE message_id = old.message_id AND kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(name, '') || ' ' || address, ' '), '')
                      FROM recipients WHERE message_id = old.message_id AND kind IN ('to', 'cc', 'bcc'))
    WHERE message_id = old.message_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_recipients_au
AFTER UPDATE ON recipients
BEGIN
    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(name, '') || ' ' || address, ' '), '')
                  FROM recipients WHERE message_id = old.message_id AND kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(name, '') || ' ' || address, ' '), '')
                      FROM recipients WHERE message_id = old.message_id AND kind IN ('to', 'cc', 'bcc'))
    WHERE message_id = old.message_id AND old.message_id IS NOT NULL;

    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(name, '') || ' ' || address, ' '), '')
                  FROM recipients WHERE message_id = new.message_id AND kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(name, '') || ' ' || address, ' '), '')
                      FROM recipients WHERE message_id = new.message_id AND kind IN ('to', 'cc', 'bcc'))
    WHERE message_id = new.message_id AND new.message_id IS NOT NULL;
END;

-- attachments -> search_documents: filenames, same recompute-from-scratch
-- approach.
CREATE TRIGGER IF NOT EXISTS trg_search_documents_attachments_ai
AFTER INSERT ON attachments
WHEN new.message_id IS NOT NULL
BEGIN
    UPDATE search_documents SET
        filenames = (SELECT coalesce(group_concat(filename, ' '), '')
                     FROM attachments WHERE message_id = new.message_id AND filename IS NOT NULL)
    WHERE message_id = new.message_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_attachments_ad
AFTER DELETE ON attachments
WHEN old.message_id IS NOT NULL
BEGIN
    UPDATE search_documents SET
        filenames = (SELECT coalesce(group_concat(filename, ' '), '')
                     FROM attachments WHERE message_id = old.message_id AND filename IS NOT NULL)
    WHERE message_id = old.message_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_attachments_au
AFTER UPDATE ON attachments
BEGIN
    UPDATE search_documents SET
        filenames = (SELECT coalesce(group_concat(filename, ' '), '')
                     FROM attachments WHERE message_id = old.message_id AND filename IS NOT NULL)
    WHERE message_id = old.message_id AND old.message_id IS NOT NULL;

    UPDATE search_documents SET
        filenames = (SELECT coalesce(group_concat(filename, ' '), '')
                     FROM attachments WHERE message_id = new.message_id AND filename IS NOT NULL)
    WHERE message_id = new.message_id AND new.message_id IS NOT NULL;
END;

-- search_documents -> messages_fts: the standard external-content sync
-- recipe (SQLite documentation, 'External Content Tables'). search_documents
-- is the only writer of messages_fts; nothing else may touch that table.
CREATE TRIGGER IF NOT EXISTS trg_messages_fts_ai
AFTER INSERT ON search_documents
BEGIN
    INSERT INTO messages_fts (rowid, sender, recipients, subject, body, filenames, list_id)
    VALUES (new.message_id, new.sender, new.recipients, new.subject, new.body, new.filenames, new.list_id);
END;

CREATE TRIGGER IF NOT EXISTS trg_messages_fts_ad
AFTER DELETE ON search_documents
BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, sender, recipients, subject, body, filenames, list_id)
    VALUES ('delete', old.message_id, old.sender, old.recipients, old.subject, old.body, old.filenames, old.list_id);
END;

CREATE TRIGGER IF NOT EXISTS trg_messages_fts_au
AFTER UPDATE ON search_documents
BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, sender, recipients, subject, body, filenames, list_id)
    VALUES ('delete', old.message_id, old.sender, old.recipients, old.subject, old.body, old.filenames, old.list_id);
    INSERT INTO messages_fts (rowid, sender, recipients, subject, body, filenames, list_id)
    VALUES (new.message_id, new.sender, new.recipients, new.subject, new.body, new.filenames, new.list_id);
END;

-- Everything that was already here.
--
-- The triggers above only see mail that arrives *after* them, and this index
-- is being retro-fitted onto stores holding tens of thousands of messages: on
-- a real account the first run is precisely the run where nothing has arrived
-- yet. Triggers alone would leave search returning nothing on every existing
-- store, for ever, which is the mistake migration 0003 made with the cached
-- mailbox counts and had to come back and fix.
--
-- `ON CONFLICT DO NOTHING` rather than a guard on the whole statement: this
-- runs on every start, and the second run has to be a cheap no-op rather than
-- a second copy of every document. The `INSERT` into `search_documents` fires
-- the FTS triggers below, so `messages_fts` follows without being touched
-- here.
INSERT INTO search_documents (message_id, subject, sender, recipients, filenames, list_id)
SELECT
    m.id,
    coalesce(m.subject, ''),
    coalesce((SELECT group_concat(coalesce(r.name, '') || ' ' || r.address, ' ')
                FROM recipients r WHERE r.message_id = m.id AND r.kind = 'from'), ''),
    coalesce((SELECT group_concat(coalesce(r.name, '') || ' ' || r.address, ' ')
                FROM recipients r
               WHERE r.message_id = m.id AND r.kind IN ('to', 'cc', 'bcc')), ''),
    coalesce((SELECT group_concat(a.filename, ' ')
                FROM attachments a
               WHERE a.message_id = m.id AND a.filename IS NOT NULL), ''),
    coalesce(m.list_id, '')
FROM messages m
-- `WHERE true` is not decoration: SQLite cannot tell an `ON CONFLICT` clause
-- from the tail of the SELECT's own WHERE without it, and rejects the
-- statement as a syntax error near `DO`.
WHERE true
ON CONFLICT (message_id) DO NOTHING;
";

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use postio_model::{Attachment, EmailAddress, Message};
    use postio_storage::repository::MessageRepository;
    use postio_storage::test_support;

    fn matches(connection: &Connection, query: &str) -> Vec<i64> {
        let mut statement = connection
            .prepare("SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?1 ORDER BY rowid")
            .expect("prepare");
        statement
            .query_map([query], |row| row.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("rows")
    }

    #[test]
    fn a_new_message_is_searchable_by_subject() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some("Quarterly report".to_string());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create message");

        assert_eq!(matches(&connection, "quarterly"), vec![message.id.get()]);
        assert!(matches(&connection, "unrelated").is_empty());
    }

    #[test]
    fn a_new_message_is_searchable_by_list_id() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.list_id = Some("harbour-dev.lists.example.org".to_string());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create message");

        // `matches` runs an unquoted MATCH; a bare hyphen has its own
        // meaning in FTS5 query syntax, so `list:`'s own value is quoted at
        // the query-builder layer instead — see `fts_literal` and
        // `list_names_a_mailing_list_by_its_list_id_not_by_a_recipient_address`
        // in `tests/executor.rs` for that path end to end.
        assert_eq!(matches(&connection, "lists"), vec![message.id.get()]);
        assert!(matches(&connection, "unrelated").is_empty());
    }

    #[test]
    fn recipients_become_searchable_sender_and_recipients_text() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.to = vec![EmailAddress::new(Some("Bob"), "bob@example.com")];
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create message");

        assert_eq!(matches(&connection, "lovelace"), vec![message.id.get()]);
        assert_eq!(matches(&connection, "bob"), vec![message.id.get()]);
    }

    #[test]
    fn attachment_filenames_become_searchable() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = Message::new(account.id, mailbox, Utc::now());
        let mut attachment = Attachment::new(message.id, "application/pdf", 1024);
        attachment.filename = Some("invoice-august.pdf".to_string());
        message.attachments = vec![attachment];
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create message");

        assert_eq!(matches(&connection, "invoice"), vec![message.id.get()]);
    }

    #[test]
    fn index_body_makes_body_text_searchable() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = Message::new(account.id, mailbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create message");

        index_body(&connection, message.id.get(), Some("the rebuild is O(n^2)")).expect("index");

        assert_eq!(matches(&connection, "rebuild"), vec![message.id.get()]);
    }

    #[test]
    fn updating_a_subject_updates_the_index() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some("Draft subject".to_string());
        let repository = MessageRepository::new(&connection);
        repository.create(&mut message).expect("create message");

        message.subject = Some("Final subject".to_string());
        repository.update(&mut message).expect("update message");

        assert!(matches(&connection, "draft").is_empty());
        assert_eq!(matches(&connection, "final"), vec![message.id.get()]);
    }

    #[test]
    fn deleting_a_message_removes_it_from_the_index() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some("Ephemeral".to_string());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create message");
        assert_eq!(matches(&connection, "ephemeral"), vec![message.id.get()]);

        MessageRepository::new(&connection)
            .delete(&[message.id])
            .expect("delete message");

        assert!(matches(&connection, "ephemeral").is_empty());
        let count: i64 = connection
            .query_row("SELECT count(*) FROM search_documents", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 0, "the shadow row must be cleaned up too");
    }

    #[test]
    fn ensure_schema_is_idempotent() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("first application");
        ensure_schema(&connection).expect("second application must be a no-op, not an error");
    }

    #[test]
    fn rebuild_restores_the_index_after_it_is_hand_emptied() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some("Rebuildable".to_string());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create message");

        connection
            .execute_batch("DELETE FROM messages_fts;")
            .expect("empty the index directly, simulating drift");
        assert!(matches(&connection, "rebuildable").is_empty());

        rebuild(&connection).expect("rebuild");

        assert_eq!(matches(&connection, "rebuildable"), vec![message.id.get()]);
    }

    #[test]
    fn index_body_on_an_unknown_message_is_a_harmless_no_op() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        ensure_schema(&connection).expect("schema");

        index_body(&connection, 999, Some("text")).expect("no-op, not an error");
    }
}
