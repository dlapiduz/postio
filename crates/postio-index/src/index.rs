//! The full-text search index: schema, sync triggers, and the rebuild path.
//!
//! # The shape: two `USING fts` indexes on ordinary tables
//!
//! The engine's full-text search is an *index method*, not a module:
//! `CREATE INDEX ... USING fts` puts an inverted index on ordinary columns,
//! and the engine maintains it the way it maintains any other index. So
//! there is no virtual table here, no external-content shadow table, and no
//! trigger whose job is keeping an index in step — all of which the FTS5
//! version of this module needed. Two indexes:
//!
//! * `search_documents_fts`, over the five metadata columns of
//!   `search_documents`;
//! * `messages_body_fts`, over `message_search_bodies.body_search` — the
//!   body text folded for search, one row per indexed message. The engine's
//!   tokenizer lowercases and does not strip diacritics, so
//!   [`postio_model::fold`] folds the column on the way in ([`index_body`])
//!   and the query path applies the identical fold, or the two stop meeting.
//!
//! # Why `search_documents` is still a table of its own
//!
//! An index covers columns, and the metadata this search covers (sender,
//! recipients, subject, attachment filenames, list id) does not live on one
//! row of `messages`: sender and recipients are rows of `recipients`,
//! filenames are rows of `attachments`. `search_documents` is the flattened
//! row — one per message — that gives the index something to sit on. It is
//! kept current by triggers on `messages`, `recipients` and `attachments`,
//! and backfilled by [`SCHEMA_FOR_TEST`]'s trailing `INSERT` for the mail
//! that predates the triggers, because a retro-fitted index that only sees
//! new arrivals answers nothing on every existing store.
//!
//! Bodies are not flattened into it: since ADR 0020 they are already one
//! column of one row (`messages.body_text`), and their index sits on a
//! sibling table of its own, `message_search_bodies`, which [`index_body`]
//! writes — the fold cannot be computed by a trigger, and keeping the folded
//! copy off `messages` keeps the index's segment merges off every other
//! write to that table.
//!
//! `search_documents.message_id` cascades from `messages.id`, so deleting a
//! message deletes its flattened row, and the indexes follow their tables
//! without any help from this crate.
//!
//! # Applying this schema
//!
//! [`ensure_schema`] runs on every start, versioned per half (metadata,
//! bodies, headers): current halves are skipped cheaply, and a mismatched
//! half is dropped and rebuilt, because the index is derived data and the
//! tables it derives from are still there (#490 is why the versions exist).
//! It is deliberately not a numbered `postio-storage` migration: this index
//! is `postio-search`'s own concern, layered on top of tables
//! `postio-storage` already created.

use postio_model::MessageBody;

use crate::error::Result;
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _, bind};

/// Creates `search_documents`, its two `USING fts` indexes and every
/// trigger that keeps them in sync, if they do not already exist.
///
/// Call this once per connection before indexing or searching — on every
/// application start, the same way the store's migrations run on every
/// start. Requires `PRAGMA foreign_keys = ON` (per connection on this
/// engine; the store's connect path sets it) so that deleting a message
/// cascades into `search_documents`.
///
/// # It indexes what is already there
///
/// Creating the triggers is only half of it. They see mail that arrives after
/// them, and this index is retro-fitted onto stores that already hold tens of
/// thousands of messages — so the schema ends with a backfill over `messages`,
/// and running it again is a no-op rather than a second copy. Everything
/// except message *bodies*: a `message_search_bodies` row is written only
/// when the extracted text exists, no trigger can derive it from raw bytes,
/// and [`index_body`] is how it arrives.
pub async fn ensure_schema(connection: &Connection) -> Result<()> {
    postio_storage::sql::batch(
        connection,
        "CREATE TABLE IF NOT EXISTS search_schema (
             half    TEXT PRIMARY KEY,
             version INTEGER NOT NULL
         );",
    )
    .await?;

    // `IF NOT EXISTS` adds new objects and cannot change an existing
    // table's columns, which is how `list_id` broke every store created
    // before it: the CREATE was a silent no-op over the old table while the
    // new triggers referenced the column it never gained (#490). So the
    // schema is versioned per half, and a mismatched half is dropped and
    // rebuilt — the index is derived data, and the mail tables it derives
    // from are exactly one `SCHEMA` run away.
    let metadata = half_version(connection, "metadata").await?;
    let bodies = half_version(connection, "bodies").await?;
    let headers = half_version(connection, "headers").await?;

    // Nothing to do, and saying so is worth more than it looks. The batch
    // below is every object `IF NOT EXISTS`, which reads as free and is not:
    // on a 20,000-message store `CREATE TRIGGER IF NOT EXISTS
    // trg_messages_fts_au` took 160-230ms with the trigger already there,
    // more than the rest of opening the store put together, on every single
    // start and growing with the mailbox (#1113). The trigger's body names
    // `messages_fts`, and parsing it appears to make SQLite instantiate the
    // fts5 module that the `CREATE VIRTUAL TABLE IF NOT EXISTS` above it
    // skips by matching on name alone.
    //
    // The versions are what make this safe to skip: they already decide
    // whether each half is current, and #490 is why they exist. The cost is
    // that a store whose objects were removed out-of-band is no longer
    // repaired by the next start -- the ordinary contract of a versioned
    // migration, and the same one `postio_storage::migrate` keeps.
    if metadata == METADATA_SCHEMA_VERSION
        && bodies == BODIES_SCHEMA_VERSION
        && headers == HEADERS_SCHEMA_VERSION
    {
        tracing::debug!("the search index schema is current");
        return Ok(());
    }

    if metadata != METADATA_SCHEMA_VERSION {
        postio_storage::sql::batch(connection, DROP_METADATA).await?;
    }
    if bodies != BODIES_SCHEMA_VERSION {
        postio_storage::sql::batch(
            connection,
            "DROP INDEX IF EXISTS messages_body_fts;
             DELETE FROM message_search_bodies;",
        )
        .await?;
    }
    if headers != HEADERS_SCHEMA_VERSION {
        // The index goes with the table; SQLite drops it either way, and
        // naming it is what stops a future rename from leaving one behind.
        postio_storage::sql::batch(
            connection,
            "DROP INDEX IF EXISTS idx_message_headers_name;
             DROP TABLE IF EXISTS message_headers;",
        )
        .await?;
    }

    postio_storage::sql::batch(connection, SCHEMA).await?;

    set_half_version(connection, "metadata", METADATA_SCHEMA_VERSION).await?;
    set_half_version(connection, "bodies", BODIES_SCHEMA_VERSION).await?;
    set_half_version(connection, "headers", HEADERS_SCHEMA_VERSION).await?;
    Ok(())
}

/// The metadata half's schema version: `search_documents`, `messages_fts`,
/// and every trigger that feeds them.
///
/// **Bump this whenever any of those changes shape** — a new column, a
/// changed trigger body, a tokenizer change. On mismatch the whole half is
/// dropped and regenerated from `messages`/`recipients`/`attachments` by
/// [`SCHEMA`]'s own backfill: one pass of SQL, no blob reads. Versions are
/// compared for *equality*, so a downgrade rebuilds too rather than leaving
/// a shape this build has never seen.
///
/// The history, so a bump has somewhere to point:
/// 1 — the original shape (#327).
/// 2 — `list_id` on `search_documents` and `messages_fts` (48a2f96).
const METADATA_SCHEMA_VERSION: i64 = 2;

/// The body half's version: `messages_body_fts` over `message_search_bodies`.
///
/// **Kept separate deliberately.** Refilling the metadata half is cheap SQL;
/// refilling this one means `index_local_bodies` re-reading every body on
/// disk — minutes on a real archive, against a search that answers
/// metadata-only until it finishes. A metadata bump must never cost that,
/// so this only moves when the body index itself changes shape. On
/// mismatch `message_search_bodies` is cleared; the catch-up pass finds
/// every message missing from it and refills in the background, batched and
/// yielding (#500).
///
/// History:
/// 1 — `messages_body_fts` on `messages.body_search`.
/// 2 — the same index moved to its own table, `message_search_bodies`, so a
///     header write no longer merges the body index (`fts_write_cost`).
const BODIES_SCHEMA_VERSION: i64 = 2;

/// The header half's version: `message_headers` and its name index.
///
/// **Bump this when the table changes shape, and also when either cap in
/// [`HEADER_ROWS_PER_MESSAGE`] or [`postio_model::headers::VALUE_LIMIT`]
/// moves** — the rows are that policy's output, so a store whose rows were
/// written under the old caps answers `header:` differently from a fresh
/// one, which is the drift ADR 0025 Q3 promises is revisable.
///
/// Revisable because refilling is cheap: unlike the body half, nothing here
/// reads a blob or decompresses a body of its own — `body_headers` is one of
/// the three columns the row already carries, and
/// [`messages_missing_header_rows`] finds every message that needs one.
const HEADERS_SCHEMA_VERSION: i64 = 1;

/// How many rows one message may contribute to `message_headers`.
///
/// ADR 0025 Q3's second cap, and the one that bounds the pathological
/// message: twenty hops through a mailing list, each adding a `Received` and
/// an ARC set, is a hundred fields for one message. The first
/// [`HEADER_ROWS_PER_MESSAGE`] in **wire order** are kept, which is where the
/// fields a person searches for live — a `Received` chain grows at the top
/// and the interesting `X-` fields sit above it.
///
/// A cap and not an allowlist, for the reason ADR 0025 Q3 gives at length: a
/// list of header names is maintained forever, lies about every name off it,
/// and converges on one provider's own vocabulary.
pub const HEADER_ROWS_PER_MESSAGE: usize = 64;

/// Everything the metadata half is made of, for the rebuild path. Kept next
/// to [`SCHEMA`], which recreates each of these: a trigger dropped here and
/// not recreated there is a column that silently stops updating.
const DROP_METADATA: &str = "DROP TRIGGER IF EXISTS trg_search_documents_messages_ai;
DROP TRIGGER IF EXISTS trg_search_documents_messages_au;
DROP TRIGGER IF EXISTS trg_search_documents_recipients_ai;
DROP TRIGGER IF EXISTS trg_search_documents_recipients_ad;
DROP TRIGGER IF EXISTS trg_search_documents_recipients_au;
DROP TRIGGER IF EXISTS trg_search_documents_attachments_ai;
DROP TRIGGER IF EXISTS trg_search_documents_attachments_ad;
DROP TRIGGER IF EXISTS trg_search_documents_attachments_au;
DROP INDEX IF EXISTS search_documents_fts;
DROP TABLE IF EXISTS search_documents;
";

/// The recorded version of one schema half, `0` when it has never been
/// recorded — a fresh store, or any store from before versioning existed.
/// Both rebuild, which for the fresh store is simply the first build.
async fn half_version(connection: &Connection, half: &str) -> Result<i64> {
    let version = sql::first(
        connection,
        "SELECT version FROM search_schema WHERE half = ?1",
        [half],
        |row| row.col(0),
    )
    .await?;
    Ok(version.unwrap_or(0))
}

async fn set_half_version(connection: &Connection, half: &str, version: i64) -> Result<()> {
    connection
        .execute(
            "INSERT INTO search_schema (half, version) VALUES (?1, ?2)
         ON CONFLICT (half) DO UPDATE SET version = excluded.version",
            bind![half, version],
        )
        .await?;
    Ok(())
}

/// Sets (or clears) the indexed body text for a message.
///
/// The raw body bytes live in the blob store, so nothing here can derive
/// this column the way the metadata columns are derived by trigger. The
/// caller reads the extracted plain-text body (E2.9's job, not this
/// crate's — raw HTML must never reach this column) and passes it here
/// once it has the bytes.
///
/// A message with no `search_documents` row yet (indexing raced ahead of the
/// message insert) is not an error: the write is simply a no-op, since there
/// is nothing to update. See [`ensure_schema`] for why the row always exists
/// once the message does.
pub async fn index_body(
    connection: &Connection,
    message_id: i64,
    body: Option<&str>,
) -> Result<()> {
    // An upsert into `message_search_bodies`, the sibling table the body
    // index is on. A body write pays the index merge here; a header write to
    // `messages` does not, which is the whole point of the table (see the
    // schema).
    //
    // **A row is written even for a message with no text**, an empty string
    // in it. The row's *presence* is the record that this message *was*
    // indexed: [`messages_missing_body_text`] asks for messages with no row
    // here, and when "tried, nothing there" left no row, every
    // attachment-only message stayed a candidate for ever -- with one batch
    // of them in a store, `postio_session::index_local_bodies` re-selected
    // the same batch in a tight loop, burning a core and streaming write
    // transactions for as long as the app ran (#500). An empty string can
    // never match, and the row can never be re-selected.
    //
    // Folded on the way in, because the engine's tokenizer will not: see
    // [`postio_model::fold`], and note that the query path must apply the
    // same fold or the two stop meeting.
    // `SELECT ... WHERE EXISTS`, not `VALUES`, so a message that is not here
    // yet is a no-op rather than a foreign-key violation. `index_body` can
    // race ahead of the message insert (the catch-up pass and a fresh sync
    // touch the same rows), and the old column write was a `WHERE id = ?`
    // `UPDATE` that simply matched nothing -- this keeps that contract now
    // that the write is an insert into a table with a foreign key.
    connection
        .execute(
            "INSERT INTO message_search_bodies (message_id, body_search)
             SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM messages WHERE id = ?1)
             ON CONFLICT (message_id) DO UPDATE SET body_search = excluded.body_search",
            (message_id, postio_model::fold::fold(body.unwrap_or(""))),
        )
        .await?;

    Ok(())
}

/// Indexes a message's body, whichever form it arrived in.
///
/// The call every producer of a body should make, rather than
/// [`index_body`] with text it extracted itself. "Raw markup must never reach
/// this column" is a rule about the column, so the crate that owns the column
/// is where it is kept: an HTML-only message goes through
/// [`postio_body::parse()`] and is indexed as what it *says*, never as its
/// markup — otherwise every such message is a hit for `div`, for `href`, and
/// for the host of every tracking redirect it carries.
///
/// `text/plain` wins when there is one. It is what the sender wrote, the
/// HTML alternative is a rendering of the same words, and indexing both would
/// double the index for no new hits.
///
/// A message with neither form clears the column rather than leaving stale
/// text behind — the same shape as `index_body(.., None)`.
pub async fn index_body_of(
    connection: &Connection,
    message_id: i64,
    body: &MessageBody,
) -> Result<()> {
    index_body(connection, message_id, indexable_text(body).as_deref()).await
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
/// `body_state` and not merely "has a blob": the column says whether the bytes
/// are on this machine, and a message the index claims to have read while its
/// body is still on the server would make search answer for a corpus it does
/// not have.
///
/// **Both `full` and `partial`.** ADR 0017 split those apart: `partial` means
/// the words are local and the attachment payloads are not, and it is the
/// settled state of every text-backfilled message carrying an attachment —
/// 15% of the reference mailbox. Asking only for `full`, which was the only
/// state a fetched body could reach when this was written, skips every one of
/// them, and the search corpus is precisely what the text axis exists to
/// complete.
///
/// Asked of `message_bodies_fts` rather than of `search_documents.body`, and
/// that matters for exactly one population, which is everybody: a store that
/// indexed its bodies before this table existed has them in the old column,
/// and asking the old column would answer "nothing to do" for every one of
/// them while the new index stayed empty for ever.
pub async fn messages_missing_body_text(connection: &Connection, limit: u32) -> Result<Vec<i64>> {
    sql::all(
        connection,
        "SELECT m.id
           FROM messages m
          WHERE m.body_state IN ('full', 'partial')
            AND NOT EXISTS (SELECT 1 FROM message_search_bodies b
                             WHERE b.message_id = m.id)
          ORDER BY m.received_at DESC
          LIMIT ?1",
        [limit],
        |row| row.col::<i64>(0),
    )
    .await
    .map_err(Into::into)
}

/// As [`messages_missing_body_text`], scoped to one account (#981).
///
/// What a per-account reindex is driven by, after
/// [`clear_account_body_index`] has made this account's own local mail the
/// candidate set — the rest of the store is untouched, so a rebuild for one
/// account never re-derives another's.
pub async fn messages_missing_body_text_for_account(
    connection: &Connection,
    account_id: i64,
    limit: u32,
) -> Result<Vec<i64>> {
    sql::all(
        connection,
        "SELECT m.id
           FROM messages m
          WHERE m.account_id = ?1
            AND m.body_state IN ('full', 'partial')
            AND NOT EXISTS (SELECT 1 FROM message_search_bodies b
                             WHERE b.message_id = m.id)
          ORDER BY m.received_at DESC
          LIMIT ?2",
        bind![account_id, limit],
        |row| row.col::<i64>(0),
    )
    .await
    .map_err(Into::into)
}

/// Removes `account_id`'s rows from `message_search_bodies`, so the next
/// [`messages_missing_body_text_for_account`] call finds them again.
///
/// Nothing here touches `messages` or `search_documents`: this account's mail
/// is not going anywhere, only its body text drops out of the index until the
/// catch-up pass puts it back. Deleting the rows is what
/// [`messages_missing_body_text_for_account`] then keys on — a message with
/// no row in `message_search_bodies` is one to index.
pub async fn clear_account_body_index(connection: &Connection, account_id: i64) -> Result<usize> {
    // Deleting the rows, not clearing a column: the body index is on
    // `message_search_bodies` now, and a missing row is what the catch-up
    // pass looks for. See `index_body` for why an indexed-but-textless
    // message holds an empty-string row.
    Ok(connection
        .execute(
            "DELETE FROM message_search_bodies
              WHERE message_id IN (SELECT id FROM messages WHERE account_id = ?1)",
            [account_id],
        )
        .await? as usize)
}

/// Replaces a message's rows in `message_headers`.
///
/// The only writer of that table. Every field becomes a row, in wire order,
/// under ADR 0025 Q3's two caps: values truncated by
/// [`postio_model::headers::normalize_value`] and the message cut off at
/// [`HEADER_ROWS_PER_MESSAGE`] fields.
///
/// # Why it normalizes rather than trusting its caller
///
/// The index holds a 512-byte prefix and an in-memory matcher (#479) holds
/// whatever it was handed, so the two disagree about every long header unless
/// both pass through the same function. Calling
/// [`Headers::normalized`](postio_model::Headers::normalized) here means a
/// caller cannot forget, and means there is one answer to "what does this
/// message's `X-Mailer` say" rather than one per evaluator.
///
/// # A message with no fields still gets a row
///
/// An empty block writes one row whose name is empty, and that is the same
/// trick [`index_body`] plays with an empty body for the same reason: the
/// rows are also the record that this message *was* indexed. Without it,
/// [`messages_missing_header_rows`] would offer a message that parses to
/// nothing on every lap for ever, which is #500 exactly. The row can never
/// match — the parser refuses an empty header name, so no `Filter::Header`
/// ever carries one.
///
/// A message with no row in `messages` is a harmless no-op rather than an
/// error, exactly as [`index_body`] is: the foreign key rejects the insert
/// and indexing that raced an expunge is not a fault.
pub async fn index_headers(
    connection: &Connection,
    message_id: i64,
    headers: &postio_model::Headers,
) -> Result<()> {
    if !message_exists(connection, message_id).await? {
        return Ok(());
    }
    // Delete first: the pass is resumable and a version bump refills the whole
    // table, so re-indexing a message is the ordinary case. An upsert would
    // leave the rows of a message that has *lost* a header behind.
    connection
        .execute(
            "DELETE FROM message_headers WHERE message_id = ?1",
            [message_id],
        )
        .await?;

    let normalized = headers.normalized();
    let mut statement = connection
        .prepare(
            "INSERT INTO message_headers (message_id, name, value, ordinal)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (message_id, name, ordinal) DO NOTHING",
        )
        .await?;
    let mut written = 0usize;
    for (ordinal, header) in normalized.iter().take(HEADER_ROWS_PER_MESSAGE).enumerate() {
        // `normalize_name` trims, so a field whose name was whitespace has
        // none. It would collide with the "indexed, nothing there" row below
        // and it is not a name anything can ask for.
        if header.name.is_empty() {
            continue;
        }
        statement
            .execute(bind![message_id, header.name, header.value, ordinal as i64])
            .await?;
        written += 1;
    }
    if written == 0 {
        statement.execute(bind![message_id, "", "", 0i64]).await?;
    }
    Ok(())
}

/// Whether `messages` still holds this id.
///
/// Asked before the delete rather than left to the foreign key, because the
/// delete on its own would succeed against no rows and the insert that
/// follows would be the thing that failed — turning "indexed a message that
/// has just been expunged" into an error the caller has to classify.
async fn message_exists(connection: &Connection, message_id: i64) -> Result<bool> {
    let found: Option<i64> = sql::first(
        connection,
        "SELECT 1 FROM messages WHERE id = ?1",
        [message_id],
        |row| row.col(0),
    )
    .await?;
    Ok(found.is_some())
}

/// Message ids whose header block is stored and whose header rows are not,
/// newest first and windowed to `limit`.
///
/// What the catch-up pass in `postio_session::index_local_headers` works
/// through: every message in every store, the first time, because nothing
/// wrote `body_headers` until #884 and nothing indexed it until now. Empty
/// on a store that is caught up, which is what makes running it on every
/// start affordable.
///
/// **Local only, and that is a scoping decision rather than an omission.**
/// `body_headers IS NOT NULL` is ADR 0025 Q5's first population — the one a
/// pass with no network can finish. A message whose block was never stored
/// belongs to `MessageRepository::messages_missing_headers` (a repair from
/// the raw blob on disk) or to `messages_needing_a_header_fetch` (the only
/// case that dials out). Mixing them here would mean batches this pass can
/// make no progress on, and the pass stops when a batch does not shrink.
pub async fn messages_missing_header_rows(connection: &Connection, limit: u32) -> Result<Vec<i64>> {
    sql::all(
        connection,
        "SELECT m.id
           FROM messages m
          WHERE m.body_headers IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM message_headers h WHERE h.message_id = m.id)
          ORDER BY m.received_at DESC
          LIMIT ?1",
        [limit],
        |row| row.col::<i64>(0),
    )
    .await
    .map_err(Into::into)
}

/// As [`messages_missing_header_rows`], scoped to one account (#981). See
/// [`messages_missing_body_text_for_account`]'s own doc for why a targeted
/// candidate query is what keeps a per-account reindex from touching any
/// other account's rows.
pub async fn messages_missing_header_rows_for_account(
    connection: &Connection,
    account_id: i64,
    limit: u32,
) -> Result<Vec<i64>> {
    sql::all(
        connection,
        "SELECT m.id
           FROM messages m
          WHERE m.account_id = ?1
            AND m.body_headers IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM message_headers h WHERE h.message_id = m.id)
          ORDER BY m.received_at DESC
          LIMIT ?2",
        bind![account_id, limit],
        |row| row.col::<i64>(0),
    )
    .await
    .map_err(Into::into)
}

/// Removes `account_id`'s rows from `message_headers`, so the next
/// [`messages_missing_header_rows_for_account`] call finds them again.
///
/// Ordinary rows, unlike [`clear_account_body_index`]'s contentless table —
/// this is the same delete [`index_headers`] itself issues before replacing
/// a message's rows, just for every message an account has rather than one.
pub async fn clear_account_header_index(connection: &Connection, account_id: i64) -> Result<usize> {
    Ok(connection
        .execute(
            "DELETE FROM message_headers
          WHERE message_id IN (SELECT id FROM messages WHERE account_id = ?1)",
            [account_id],
        )
        .await? as usize)
}

/// Rebuilds the metadata index from `search_documents`.
///
/// Dropped and recreated, which is what "rebuild" means for an index. It was
/// FTS5's own `'rebuild'` command -- a message to a virtual table, telling it
/// to regenerate its b-trees from its content table. There is no virtual
/// table to send a message to now: the index is an ordinary index on an
/// ordinary table, and the engine builds it from the rows the same way it
/// would any other.
///
/// Use it after a bulk import that bypassed the triggers, or as a maintenance
/// operation if the index is ever suspected of having drifted.
///
/// It does **not** rebuild the body index, and does not need to: that one is
/// on `messages.body_search`, a column of the same table, and is maintained
/// by the same writes. What catches a body up is
/// [`messages_missing_body_text`] and the pass behind it.
pub async fn rebuild(connection: &Connection) -> Result<()> {
    postio_storage::sql::batch(
        connection,
        "DROP INDEX IF EXISTS search_documents_fts;
             CREATE INDEX search_documents_fts ON search_documents
                 USING fts (sender, recipients, subject, filenames, list_id);",
    )
    .await?;
    Ok(())
}

/// The schema text, for a test that applies it one statement at a time.
pub const SCHEMA_FOR_TEST: &str = SCHEMA;

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS search_documents (
    message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    sender      TEXT NOT NULL DEFAULT '',
    recipients  TEXT NOT NULL DEFAULT '',
    subject     TEXT NOT NULL DEFAULT '',
    filenames   TEXT NOT NULL DEFAULT '',
    list_id     TEXT NOT NULL DEFAULT ''
);

-- The metadata index: an index on the table, not a table beside it.
--
-- This was `messages_fts`, an FTS5 external-content virtual table, with three
-- triggers keeping it in step with `search_documents`. The engine's full-text
-- search is an *index method* rather than a module, so the shadow table and
-- all three of its triggers are gone: the engine maintains this the way it
-- maintains any other index, and there is no second copy of anything to keep
-- in step.
--
-- The five columns are one index rather than five, because `fts_match` takes
-- the columns it is searching and the planner resolves the set to an index
-- that covers them.
CREATE INDEX IF NOT EXISTS search_documents_fts ON search_documents
    USING fts (sender, recipients, subject, filenames, list_id);

-- The body index, over its own table (`postio_storage::schema` defines
-- `message_search_bodies`; this owns the index on it).
--
-- `body_search` is `messages.body_text` folded for search. It sits in a
-- sibling table rather than on `messages` so a header write does not merge
-- the body index -- the table's own documentation in `postio_storage::schema`
-- has the write-cost measurement.
--
-- `postio_model::fold` does the folding, and the query path applies the
-- identical fold -- the engine's tokenizer lowercases and does not strip
-- diacritics, so an unaccented query finds an accented word only because
-- both sides went through the same fold. `index_body` is the one writer,
-- run by `postio_session::spawn_body_indexer` in batches off the sync lane;
-- a stored body with no row here is what the indexer has yet to reach.
CREATE INDEX IF NOT EXISTS messages_body_fts ON message_search_bodies USING fts (body_search);

-- Arbitrary headers: the table `header:` matches against (ADR 0025 Q2).
--
-- An ordinary table rather than a second full-text index, and the reason is
-- the values: `header:` is wanted for `x-mailer=mutt`, `spf=pass`,
-- `multipart/signed`, `1.5.24`. A word tokenizer splits every one of those
-- into pieces and loses the adjacency that made it meaningful, so this is a
-- substring match on a column, not an inverted index.
--
-- No longer `WITHOUT ROWID`: the engine puts that behind an experimental flag
-- and will not build a secondary index on such a table, and
-- `idx_message_headers_name` is what makes `header:` a range scan over one
-- name rather than a scan of everything.
CREATE TABLE IF NOT EXISTS message_headers (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,   -- lowercased; RFC 5322 names are case-insensitive
    value      TEXT    NOT NULL,   -- unfolded, RFC 2047-decoded, truncated at VALUE_LIMIT
    ordinal    INTEGER NOT NULL,   -- position within the message, wire order
    PRIMARY KEY (message_id, name, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_message_headers_name ON message_headers (name, message_id);

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
        sender = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                  FROM recipients r JOIN addresses a ON a.id = r.address_id
                 WHERE r.message_id = new.message_id AND r.kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                      FROM recipients r JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = new.message_id AND r.kind IN ('to', 'cc', 'bcc'))
    WHERE message_id = new.message_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_recipients_ad
AFTER DELETE ON recipients
WHEN old.message_id IS NOT NULL
BEGIN
    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                  FROM recipients r JOIN addresses a ON a.id = r.address_id
                 WHERE r.message_id = old.message_id AND r.kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                      FROM recipients r JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = old.message_id AND r.kind IN ('to', 'cc', 'bcc'))
    WHERE message_id = old.message_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_recipients_au
AFTER UPDATE ON recipients
BEGIN
    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                  FROM recipients r JOIN addresses a ON a.id = r.address_id
                 WHERE r.message_id = old.message_id AND r.kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                      FROM recipients r JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = old.message_id AND r.kind IN ('to', 'cc', 'bcc'))
    WHERE message_id = old.message_id AND old.message_id IS NOT NULL;

    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                  FROM recipients r JOIN addresses a ON a.id = r.address_id
                 WHERE r.message_id = new.message_id AND r.kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                      FROM recipients r JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = new.message_id AND r.kind IN ('to', 'cc', 'bcc'))
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
-- a second copy of every document. The index on `search_documents` follows
-- the rows without being touched here -- it is an index, and the engine
-- maintains it the way it maintains any other.
INSERT INTO search_documents (message_id, subject, sender, recipients, filenames, list_id)
SELECT
    m.id,
    coalesce(m.subject, ''),
    coalesce((SELECT group_concat(coalesce(r.name, '') || ' ' || a.address, ' ')
                FROM recipients r JOIN addresses a ON a.id = r.address_id
               WHERE r.message_id = m.id AND r.kind = 'from'), ''),
    coalesce((SELECT group_concat(coalesce(r.name, '') || ' ' || a.address, ' ')
                FROM recipients r JOIN addresses a ON a.id = r.address_id
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

    /// Metadata matches, through the index rather than through a virtual
    /// table: `fts_match` takes the columns it is searching, and the planner
    /// resolves that set to `search_documents_fts`.
    async fn matches(connection: &Connection, query: &str) -> Vec<i64> {
        sql::all(
            connection,
            "SELECT message_id FROM search_documents
              WHERE fts_match(sender, recipients, subject, filenames, list_id, ?1)
              ORDER BY message_id",
            [query],
            |row| row.col(0),
        )
        .await
        .expect("query")
    }

    #[tokio::test]
    async fn a_new_message_is_searchable_by_subject() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some("Quarterly report".to_string());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create message");

        assert_eq!(
            matches(&connection, "quarterly").await,
            vec![message.id.get()]
        );
        assert!(matches(&connection, "unrelated").await.is_empty());
    }

    #[tokio::test]
    async fn a_new_message_is_searchable_by_list_id() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.list_id = Some("harbour-dev.lists.example.org".to_string());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create message");

        // `matches` runs an unquoted MATCH; a bare hyphen has its own
        // meaning in FTS5 query syntax, so `list:`'s own value is quoted at
        // the query-builder layer instead — see `fts_literal` and
        // `list_names_a_mailing_list_by_its_list_id_not_by_a_recipient_address`
        // in `tests/executor.rs` for that path end to end.
        assert_eq!(matches(&connection, "lists").await, vec![message.id.get()]);
        assert!(matches(&connection, "unrelated").await.is_empty());
    }

    #[tokio::test]
    async fn recipients_become_searchable_sender_and_recipients_text() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.to = vec![EmailAddress::new(Some("Bob"), "bob@example.com")];
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create message");

        assert_eq!(
            matches(&connection, "lovelace").await,
            vec![message.id.get()]
        );
        assert_eq!(matches(&connection, "bob").await, vec![message.id.get()]);
    }

    #[tokio::test]
    async fn attachment_filenames_become_searchable() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = Message::new(account.id, mailbox, Utc::now());
        let mut attachment = Attachment::new(message.id, "application/pdf", 1024);
        attachment.filename = Some("invoice-august.pdf".to_string());
        message.attachments = vec![attachment];
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create message");

        assert_eq!(
            matches(&connection, "invoice").await,
            vec![message.id.get()]
        );
    }

    /// Body matches, which are an index on `messages.body_search` now (#407,
    /// `specs/004-turso-store`) rather than a contentless table of their own.
    ///
    /// The query goes through the same fold the write path applied, which is
    /// the rule `postio_model::fold` exists to keep: both sides or neither.
    async fn body_matches(connection: &Connection, query: &str) -> Vec<i64> {
        let folded = postio_model::fold::fold(query);
        sql::all(
            connection,
            "SELECT message_id FROM message_search_bodies              WHERE fts_match(body_search, ?1) ORDER BY message_id",
            [folded.as_str()],
            |row| row.col(0),
        )
        .await
        .expect("query")
    }

    #[tokio::test]
    async fn index_body_makes_body_text_searchable() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = Message::new(account.id, mailbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create message");

        index_body(&connection, message.id.get(), Some("the rebuild is O(n^2)"))
            .await
            .expect("index");

        assert_eq!(
            body_matches(&connection, "rebuild").await,
            vec![message.id.get()]
        );
        assert!(
            matches(&connection, "rebuild").await.is_empty(),
            "and not in the metadata index, which no longer carries bodies"
        );
    }

    #[tokio::test]
    async fn updating_a_subject_updates_the_index() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some("Draft subject".to_string());
        let repository = MessageRepository::new(&connection);
        repository
            .create(&mut message)
            .await
            .expect("create message");

        message.subject = Some("Final subject".to_string());
        repository
            .update(&mut message)
            .await
            .expect("update message");

        assert!(matches(&connection, "draft").await.is_empty());
        assert_eq!(matches(&connection, "final").await, vec![message.id.get()]);
    }

    #[tokio::test]
    async fn deleting_a_message_removes_it_from_the_index() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some("Ephemeral".to_string());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create message");
        assert_eq!(
            matches(&connection, "ephemeral").await,
            vec![message.id.get()]
        );

        MessageRepository::new(&connection)
            .delete(&[message.id])
            .await
            .expect("delete message");

        assert!(matches(&connection, "ephemeral").await.is_empty());
        let count: i64 = sql::one(
            &connection,
            "SELECT count(*) FROM search_documents",
            (),
            |row| row.col(0),
        )
        .await
        .expect("count");
        assert_eq!(count, 0, "the shadow row must be cleaned up too");
    }

    #[tokio::test]
    async fn ensure_schema_is_idempotent() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("first application");
        ensure_schema(&connection)
            .await
            .expect("second application must be a no-op, not an error");
    }

    #[tokio::test]
    async fn rebuild_restores_the_index_after_it_is_hand_emptied() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some("Rebuildable".to_string());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create message");

        // Drift, simulated the only way an index method allows: drop the
        // index. There is no shadow table to empty -- which is itself the
        // point of the change, since the drift this test was written for
        // (#407) was a shadow table falling out of step with its content.
        postio_storage::sql::batch(&connection, "DROP INDEX search_documents_fts;")
            .await
            .expect("drop the index, simulating drift");

        rebuild(&connection).await.expect("rebuild");

        assert_eq!(
            matches(&connection, "rebuildable").await,
            vec![message.id.get()]
        );
    }

    #[tokio::test]
    async fn index_body_on_an_unknown_message_is_a_harmless_no_op() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");

        index_body(&connection, 999, Some("text"))
            .await
            .expect("no-op, not an error");
    }

    /// A second account in the same store, for the scoping tests below.
    async fn second_account(
        connection: &Connection,
    ) -> (postio_model::Account, postio_model::ids::MailboxId) {
        let mut account = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        postio_storage::repository::AccountRepository::new(connection)
            .create(&mut account)
            .await
            .expect("second account");
        let mailbox = postio_storage::test_support::mailbox(connection, &account, "INBOX").await;
        (account, mailbox.id)
    }

    #[tokio::test]
    async fn clearing_and_recandidating_a_bodys_index_touches_only_that_account() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (first, first_inbox) = test_support::account_with_inbox(&connection).await;
        let (second, second_inbox) = second_account(&connection).await;

        let messages = MessageRepository::new(&connection);
        let mut a = Message::new(first.id, first_inbox, Utc::now());
        a.sync.body_state = postio_model::BodyState::Full;
        messages.create(&mut a).await.expect("create a");
        index_body_of(
            &connection,
            a.id.get(),
            &MessageBody {
                text: Some("alpha".to_owned()),
                html: None,
            },
        )
        .await
        .expect("index a's body");

        let mut b = Message::new(second.id, second_inbox, Utc::now());
        b.sync.body_state = postio_model::BodyState::Full;
        messages.create(&mut b).await.expect("create b");
        index_body_of(
            &connection,
            b.id.get(),
            &MessageBody {
                text: Some("beta".to_owned()),
                html: None,
            },
        )
        .await
        .expect("index b's body");

        // Both are indexed, so neither is a candidate for either account yet.
        assert!(
            messages_missing_body_text_for_account(&connection, first.id.get(), 10)
                .await
                .expect("candidates")
                .is_empty()
        );

        let cleared = clear_account_body_index(&connection, first.id.get())
            .await
            .expect("clear");
        assert_eq!(cleared, 1, "only the first account's one message");

        assert_eq!(
            messages_missing_body_text_for_account(&connection, first.id.get(), 10)
                .await
                .expect("candidates"),
            vec![a.id.get()],
            "the cleared account's message is a candidate again"
        );
        assert!(
            messages_missing_body_text_for_account(&connection, second.id.get(), 10)
                .await
                .expect("candidates")
                .is_empty(),
            "the other account's index was never touched"
        );
    }

    #[tokio::test]
    async fn clearing_and_recandidating_headers_touches_only_that_account() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (first, first_inbox) = test_support::account_with_inbox(&connection).await;
        let (second, second_inbox) = second_account(&connection).await;

        let messages = MessageRepository::new(&connection);
        let block = postio_model::headers::Block {
            text: "Subject: hi".to_owned(),
            truncated: false,
        };
        let mut a = Message::new(first.id, first_inbox, Utc::now());
        messages.create(&mut a).await.expect("create a");
        messages
            .set_headers(a.id, Some(&block))
            .await
            .expect("store a's block");
        index_headers(&connection, a.id.get(), &postio_model::Headers::default())
            .await
            .expect("index a's headers");

        let mut b = Message::new(second.id, second_inbox, Utc::now());
        messages.create(&mut b).await.expect("create b");
        messages
            .set_headers(b.id, Some(&block))
            .await
            .expect("store b's block");
        index_headers(&connection, b.id.get(), &postio_model::Headers::default())
            .await
            .expect("index b's headers");

        let cleared = clear_account_header_index(&connection, first.id.get())
            .await
            .expect("clear");
        assert_eq!(cleared, 1, "only the first account's one message");

        assert_eq!(
            messages_missing_header_rows_for_account(&connection, first.id.get(), 10)
                .await
                .expect("candidates"),
            vec![a.id.get()],
            "the cleared account's message is a candidate again"
        );
        assert!(
            messages_missing_header_rows_for_account(&connection, second.id.get(), 10)
                .await
                .expect("candidates")
                .is_empty(),
            "the other account's index was never touched"
        );
    }
}
