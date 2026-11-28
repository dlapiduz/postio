//! The full-text search index: two `USING fts` indexes on ordinary tables, `search_documents_fts`
//! over five metadata columns and `messages_body_fts` over `message_search_bodies.body_search`.
//! The tokenizer does not strip diacritics, so [`postio_model::fold`] folds the body column on the
//! way in ([`index_body`]) and the query path applies the identical fold.
//! `search_documents` is a flattened row per message kept current by triggers, since sender,
//! recipients and filenames live in other tables; [`ensure_schema`] is versioned per half, not a
//! `postio-storage` migration.

use postio_model::MessageBody;

use crate::error::Result;
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _, bind};

/// Call once per connection before indexing or searching, on every start. Requires `PRAGMA
/// foreign_keys = ON` so deleting a message cascades into `search_documents`.
/// The schema ends with a backfill over `messages`, so a store that predates the triggers is
/// indexed and a rerun is a no-op; bodies arrive through [`index_body`], since no trigger can
/// derive them.
pub async fn ensure_schema(connection: &Connection) -> Result<()> {
    postio_storage::sql::batch(
        connection,
        "CREATE TABLE IF NOT EXISTS search_schema (
             half    TEXT PRIMARY KEY,
             version INTEGER NOT NULL
         );",
    )
    .await?;

    // `IF NOT EXISTS` cannot change an existing table's columns, which is how `list_id` broke every
    // store created before it (#490):
    // the schema is versioned per half and a mismatched half is dropped and rebuilt, since the
    // index is derived data.
    crate::events::ensure(connection).await?;
    let metadata = half_version(connection, "metadata").await?;
    let bodies = half_version(connection, "bodies").await?;
    let headers = half_version(connection, "headers").await?;
    let attachments = half_version(connection, "attachments").await?;
    let extractor = half_version(connection, "extractor").await?;

    // Skipped when current: on a 20,000-message store `CREATE TRIGGER IF NOT EXISTS` took 160-230
    // ms even with the trigger present, on every start (#1113).
    // The cost is that objects removed out-of-band are no longer repaired by the next start, the
    // ordinary contract of a versioned migration.
    if metadata == METADATA_SCHEMA_VERSION
        && bodies == BODIES_SCHEMA_VERSION
        && headers == HEADERS_SCHEMA_VERSION
        && attachments == ATTACHMENTS_SCHEMA_VERSION
        && extractor == i64::from(postio_extract::EXTRACTOR_VERSION)
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
        postio_storage::sql::batch(
            connection,
            "DROP TRIGGER IF EXISTS trg_message_contents_header_index_au;
             DROP INDEX IF EXISTS idx_message_headers_name;
             DROP TABLE IF EXISTS message_headers;",
        )
        .await?;
    }

    if attachments != ATTACHMENTS_SCHEMA_VERSION {
        postio_storage::sql::batch(connection, DROP_ATTACHMENTS).await?;
    }

    postio_storage::sql::batch(connection, SCHEMA).await?;
    postio_storage::sql::batch(connection, BODY_INDEX).await?;
    postio_storage::sql::batch(connection, ATTACHMENTS_SCHEMA).await?;

    set_half_version(connection, "metadata", METADATA_SCHEMA_VERSION).await?;
    set_half_version(connection, "bodies", BODIES_SCHEMA_VERSION).await?;
    set_half_version(connection, "headers", HEADERS_SCHEMA_VERSION).await?;
    set_half_version(connection, "attachments", ATTACHMENTS_SCHEMA_VERSION).await?;
    if attachments != ATTACHMENTS_SCHEMA_VERSION
        || extractor != i64::from(postio_extract::EXTRACTOR_VERSION)
    {
        set_half_version(
            connection,
            "extractor",
            i64::from(postio_extract::EXTRACTOR_VERSION),
        )
        .await?;
        postio_storage::sql::batch(connection, RECOUNT_OWED).await?;
    }
    Ok(())
}

/// Bump whenever the metadata half changes shape: a mismatch drops and regenerates it from
/// `messages`/`recipients`/`attachments` by [`SCHEMA`]'s backfill, no blob reads.
/// Versions compare for equality, so a downgrade rebuilds too.
const METADATA_SCHEMA_VERSION: i64 = 4;

/// Kept separate from the metadata version: refilling this half re-reads every body on disk
/// (minutes on a real archive), so a metadata bump must never cost it (#500).
/// On mismatch `message_search_bodies` is cleared and the catch-up pass refills it.
const BODIES_SCHEMA_VERSION: i64 = 3;

/// Bump when the table changes shape, or when either cap ([`HEADER_ROWS_PER_MESSAGE`],
/// [`postio_model::headers::VALUE_LIMIT`]) moves: the rows are that policy's output (ADR 0025 Q3).
/// Refilling is cheap, since it reads no blob.
const HEADERS_SCHEMA_VERSION: i64 = 2;

/// Bump when either table or the location encoding ([`encode_location`]) changes. A change in what
/// the extractor produces is
/// `postio_extract::EXTRACTOR_VERSION`'s to say: older rows are simply missing again
/// ([`attachments_missing_text`]).
pub const ATTACHMENTS_SCHEMA_VERSION: i64 = 3;

/// The first rows in wire order are kept, where the fields a person searches for live. A cap, not
/// an allowlist, which is maintained forever and converges on one provider's vocabulary (ADR 0025
/// Q3).
pub const HEADER_ROWS_PER_MESSAGE: usize = 64;

/// Kept next to [`SCHEMA`], which recreates each of these: a trigger dropped here and not recreated
/// there silently stops updating.
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
DROP TABLE IF EXISTS search_documents_deferred;
";

const DROP_ATTACHMENTS: &str = "DROP TRIGGER IF EXISTS trg_attachment_text_owed_attachments_ai;
DROP TRIGGER IF EXISTS trg_attachment_text_owed_attachments_au;
DROP TRIGGER IF EXISTS trg_attachment_text_owed_attachments_ad;
DROP TRIGGER IF EXISTS trg_attachment_text_owed_messages_au;
DROP TRIGGER IF EXISTS trg_attachment_text_owed_messages_ad;
DROP TRIGGER IF EXISTS trg_attachment_text_owed_extraction_ai;
DROP TRIGGER IF EXISTS trg_attachment_text_owed_extraction_au;
DROP TRIGGER IF EXISTS trg_attachment_text_owed_extraction_ad;
DROP TABLE IF EXISTS attachment_text_owed;
DROP INDEX IF EXISTS attachment_passages_fts;
DROP INDEX IF EXISTS idx_attachment_passages_part;
DROP TABLE IF EXISTS attachment_passages;
DROP TABLE IF EXISTS attachment_extraction;
";

/// Keyed by content, not by the attachment row: an `attachments` projection is replaced by a
/// refetch, so text keyed by its id would be extracted again on every refetch.
/// `(content_id, position)` gives one extraction however many folders hold the message (#1805).
/// Evicting a blob does not remove its text: it is derived, small (`Limits::max_text`), and lives
/// in the encrypted store.
const ATTACHMENTS_SCHEMA: &str = "\
-- One located unit of an attachment's text: a PDF page, a sheet row, a
-- slide, a paragraph, a table row, a line. `text` is as extracted, for
-- passages; `text_search` is it folded (postio_model::fold), for the index,
-- because the fold is not reversible.
CREATE TABLE IF NOT EXISTS attachment_passages (
    id           INTEGER PRIMARY KEY,
    content_id   INTEGER NOT NULL REFERENCES message_contents(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL,
    ordinal      INTEGER NOT NULL,
    location     TEXT    NOT NULL,
    text         TEXT    NOT NULL,
    text_search  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attachment_passages_part
    ON attachment_passages (content_id, position, ordinal);
CREATE INDEX IF NOT EXISTS attachment_passages_fts ON attachment_passages USING fts (text_search);

-- Which attachments have been tried, by which extractor, and how it went.
-- A row's presence is the record that the part was tried -- an empty or
-- failed extraction included -- so the indexer's queue, \"downloaded and
-- no current row here\", never offers the same part twice (#500).
CREATE TABLE IF NOT EXISTS attachment_extraction (
    content_id  INTEGER NOT NULL REFERENCES message_contents(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    version     INTEGER NOT NULL,
    -- `unavailable` is a skip that is not final: the part was not read
    -- because the reader it needs was missing (`Skip::Unavailable`), and
    -- the indexer takes the row away once it is there
    -- ([`requeue_unavailable`]).
    outcome     TEXT    NOT NULL
                CHECK (outcome IN ('complete','truncated','skipped','unavailable','failed')),
    units       INTEGER NOT NULL,
    PRIMARY KEY (content_id, position)
);

-- The parts the search still owes a reading: exactly the indexer's queue
-- (`MISSING`) as a set of (content, position), kept by the triggers below
-- so that \"has every downloaded attachment been read?\" -- asked on every
-- search -- is one seek, where asking the queue walks every attachment on
-- the machine whenever the answer is yes. ADR 0040's rule: an aggregate on
-- a hot path is a maintained answer, not a walk.
--
-- \"This extractor\" is `search_schema`'s `extractor` row, which
-- `ensure_schema` keeps at `postio_extract::EXTRACTOR_VERSION` and recounts
-- this table against when it moves.
CREATE TABLE IF NOT EXISTS attachment_text_owed (
    content_id  INTEGER NOT NULL REFERENCES message_contents(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    PRIMARY KEY (content_id, position)
);

-- A part comes onto this machine: owed, unless this extractor has read it.
CREATE TRIGGER IF NOT EXISTS trg_attachment_text_owed_attachments_ai
AFTER INSERT ON attachments
WHEN new.message_id IS NOT NULL AND new.blob_id IS NOT NULL
BEGIN
    INSERT INTO attachment_text_owed (content_id, position)
    SELECT m.content_id, new.position FROM messages m
     WHERE m.id = new.message_id AND m.content_id IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM attachment_extraction e
                        WHERE e.content_id = m.content_id AND e.position = new.position
                          AND e.version = (SELECT version FROM search_schema
                                            WHERE half = 'extractor'))
    ON CONFLICT DO NOTHING;
END;

-- Downloaded, or evicted. An eviction forgets the part only when no other
-- occurrence of the payload still has its bytes here.
CREATE TRIGGER IF NOT EXISTS trg_attachment_text_owed_attachments_au
AFTER UPDATE OF blob_id ON attachments
WHEN new.message_id IS NOT NULL AND (old.blob_id IS NULL) <> (new.blob_id IS NULL)
BEGIN
    INSERT INTO attachment_text_owed (content_id, position)
    SELECT m.content_id, new.position FROM messages m
     WHERE new.blob_id IS NOT NULL
       AND m.id = new.message_id AND m.content_id IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM attachment_extraction e
                        WHERE e.content_id = m.content_id AND e.position = new.position
                          AND e.version = (SELECT version FROM search_schema
                                            WHERE half = 'extractor'))
    ON CONFLICT DO NOTHING;
    DELETE FROM attachment_text_owed
     WHERE new.blob_id IS NULL
       AND content_id = (SELECT content_id FROM messages WHERE id = new.message_id)
       AND position NOT IN (
           SELECT a.position FROM messages m JOIN attachments a ON a.message_id = m.id
            WHERE m.content_id = (SELECT content_id FROM messages WHERE id = new.message_id)
              AND a.blob_id IS NOT NULL);
END;

CREATE TRIGGER IF NOT EXISTS trg_attachment_text_owed_attachments_ad
AFTER DELETE ON attachments
WHEN old.message_id IS NOT NULL AND old.blob_id IS NOT NULL
BEGIN
    DELETE FROM attachment_text_owed
     WHERE content_id = (SELECT content_id FROM messages WHERE id = old.message_id)
       AND position NOT IN (
           SELECT a.position FROM messages m JOIN attachments a ON a.message_id = m.id
            WHERE m.content_id = (SELECT content_id FROM messages WHERE id = old.message_id)
              AND a.blob_id IS NOT NULL);
END;

-- An occurrence moves to another payload: its parts on disk are owed for
-- the new one, and the old one keeps only what another occurrence holds.
CREATE TRIGGER IF NOT EXISTS trg_attachment_text_owed_messages_au
AFTER UPDATE OF content_id ON messages
WHEN old.content_id IS NOT new.content_id
BEGIN
    INSERT INTO attachment_text_owed (content_id, position)
    SELECT new.content_id, a.position FROM attachments a
     WHERE a.message_id = new.id AND a.blob_id IS NOT NULL AND new.content_id IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM attachment_extraction e
                        WHERE e.content_id = new.content_id AND e.position = a.position
                          AND e.version = (SELECT version FROM search_schema
                                            WHERE half = 'extractor'))
    ON CONFLICT DO NOTHING;
    DELETE FROM attachment_text_owed
     WHERE content_id = old.content_id
       AND position NOT IN (
           SELECT a.position FROM messages m JOIN attachments a ON a.message_id = m.id
            WHERE m.content_id = old.content_id AND a.blob_id IS NOT NULL);
END;

-- An occurrence goes. Its attachment rows may go before or after it; by
-- the time this runs the message is not there to be joined through, so
-- what is left is what the other occurrences hold.
CREATE TRIGGER IF NOT EXISTS trg_attachment_text_owed_messages_ad
AFTER DELETE ON messages
WHEN old.content_id IS NOT NULL
BEGIN
    DELETE FROM attachment_text_owed
     WHERE content_id = old.content_id
       AND position NOT IN (
           SELECT a.position FROM messages m JOIN attachments a ON a.message_id = m.id
            WHERE m.content_id = old.content_id AND a.blob_id IS NOT NULL);
END;

-- Read by this extractor: settled. By another: owed again, if its bytes are
-- here -- a row is only ever written at the current version, so this is a
-- store from an older build, or a test aging its rows.
CREATE TRIGGER IF NOT EXISTS trg_attachment_text_owed_extraction_ai
AFTER INSERT ON attachment_extraction
BEGIN
    DELETE FROM attachment_text_owed
     WHERE content_id = new.content_id AND position = new.position
       AND new.version = (SELECT version FROM search_schema WHERE half = 'extractor');
END;

CREATE TRIGGER IF NOT EXISTS trg_attachment_text_owed_extraction_au
AFTER UPDATE ON attachment_extraction
BEGIN
    DELETE FROM attachment_text_owed
     WHERE content_id = new.content_id AND position = new.position
       AND new.version = (SELECT version FROM search_schema WHERE half = 'extractor');
    INSERT INTO attachment_text_owed (content_id, position)
    SELECT new.content_id, new.position
     WHERE new.version IS NOT (SELECT version FROM search_schema WHERE half = 'extractor')
       AND EXISTS (SELECT 1 FROM messages m JOIN attachments a ON a.message_id = m.id
                    WHERE m.content_id = new.content_id AND a.position = new.position
                      AND a.blob_id IS NOT NULL)
    ON CONFLICT DO NOTHING;
END;

-- A record taken away puts the part back in the queue, if it is on disk.
CREATE TRIGGER IF NOT EXISTS trg_attachment_text_owed_extraction_ad
AFTER DELETE ON attachment_extraction
BEGIN
    INSERT INTO attachment_text_owed (content_id, position)
    SELECT old.content_id, old.position
     WHERE EXISTS (SELECT 1 FROM messages m JOIN attachments a ON a.message_id = m.id
                    WHERE m.content_id = old.content_id AND a.position = old.position
                      AND a.blob_id IS NOT NULL)
    ON CONFLICT DO NOTHING;
END;
";

const RECOUNT_OWED: &str = "DELETE FROM attachment_text_owed;
INSERT INTO attachment_text_owed (content_id, position)
SELECT DISTINCT m.content_id, a.position
  FROM attachments a JOIN messages m ON m.id = a.message_id
 WHERE a.blob_id IS NOT NULL AND m.content_id IS NOT NULL
   AND NOT EXISTS (SELECT 1 FROM attachment_extraction e
                    WHERE e.content_id = m.content_id AND e.position = a.position
                      AND e.version = (SELECT version FROM search_schema
                                        WHERE half = 'extractor'))
ON CONFLICT DO NOTHING;
";

/// `0` when never recorded; a fresh store and a pre-versioning one both rebuild.
pub(crate) async fn half_version(connection: &Connection, half: &str) -> Result<i64> {
    let version = sql::first(
        connection,
        "SELECT version FROM search_schema WHERE half = ?1",
        [half],
        |row| row.col(0),
    )
    .await?;
    Ok(version.unwrap_or(0))
}

pub(crate) async fn set_half_version(
    connection: &Connection,
    half: &str,
    version: i64,
) -> Result<()> {
    connection
        .execute(
            "INSERT INTO search_schema (half, version) VALUES (?1, ?2)
         ON CONFLICT (half) DO UPDATE SET version = excluded.version",
            bind![half, version],
        )
        .await?;
    Ok(())
}

/// Raw markup must never reach this column, so the caller passes extracted plain text. A message
/// with no `search_documents` row yet (indexing raced ahead of the insert) is a no-op.
pub async fn index_body(
    connection: &Connection,
    message_id: i64,
    body: Option<&str>,
) -> Result<()> {
    // A row is written even for a message with no text (an empty string): its presence records that
    // it was indexed, and without it every
    // attachment-only message stayed a candidate for ever, re-selected in a tight loop (#500).
    // Folded on the way in, because the tokenizer will not ([`postio_model::fold`]).
    // `SELECT ... WHERE EXISTS`, not `VALUES`, so a message not here yet is a no-op rather than a
    // foreign-key violation.
    connection
        .execute(
            "INSERT INTO message_search_bodies (content_id, body_search)
             SELECT content_id, ?2 FROM messages WHERE id = ?1
             ON CONFLICT (content_id) DO UPDATE SET body_search = excluded.body_search",
            (message_id, postio_model::fold::fold(body.unwrap_or(""))),
        )
        .await?;

    Ok(())
}

/// Each body written into an indexed column pays the index's merge (18 of the 24 seconds the search
/// seed's five hundred bodies took, #2020), so the index is dropped,
/// rows are written plain in one transaction, and the index is rebuilt even when a write fails. For
/// a store nothing else is searching while it fills.
pub async fn index_bodies(connection: &Connection, bodies: &[(i64, &str)]) -> Result<()> {
    postio_storage::sql::batch(connection, "DROP INDEX IF EXISTS messages_body_fts").await?;
    let written = postio_storage::sql::in_scope(connection, |connection| async move {
        for (message_id, body) in bodies {
            index_body(&connection, *message_id, Some(body)).await?;
        }
        Ok::<_, crate::error::Error>(())
    })
    .await;
    postio_storage::sql::batch(connection, BODY_INDEX).await?;
    written
}

/// The call every producer of a body should make: an HTML-only message is indexed as what it says,
/// never as its markup, or every such message is a hit for `div`.
/// `text/plain` wins when there is one, since indexing both would double the index for no new hits.
/// A message with neither clears the column.
pub async fn index_body_of(
    connection: &Connection,
    message_id: i64,
    body: &MessageBody,
) -> Result<()> {
    index_body(connection, message_id, indexable_text(body).as_deref()).await
}

/// Separate from [`index_body_of`] so the maintenance pass and the sync path provably agree on what
/// a message's indexable text is.
pub fn indexable_text(body: &MessageBody) -> Option<String> {
    if let Some(text) = body.text.as_deref().filter(|text| !text.trim().is_empty()) {
        return Some(text.to_owned());
    }
    let html = body.html.as_deref()?;
    // `to_search_text`, not `to_text`: the latter spells a link out as `label <href>` for quoted
    // replies, which an index must not.
    let text = postio_body::parse(html).to_search_text();
    (!text.trim().is_empty()).then_some(text)
}

/// Resumable from a cursor: ordered by `received_at DESC`, the order a sweep fills the index in, so
/// without one batch k walks past everything earlier batches indexed and a sweep is quadratic
/// (#1549).
/// Includes both `full` and `partial` bodies (ADR 0017: `partial` is the settled state of every
/// text-backfilled message with an attachment), and tests `body_state` rather than "has a blob", so
/// search never answers for a corpus it does not have.
/// Empty on a caught-up store, which makes running it on every start affordable.
pub async fn messages_missing_body_text(
    connection: &Connection,
    limit: u32,
    after: Option<Candidate>,
) -> Result<Vec<Candidate>> {
    // Two statements, not one with a nullable cursor: the planner sees a literal range on the
    // leading index column in the second.
    match after {
        None => sql::all(
            connection,
            "SELECT m.id, m.received_at
               FROM messages m
              WHERE m.body_state IN ('full', 'partial')
                AND m.id = (SELECT min(id) FROM messages WHERE content_id = m.content_id AND body_state IN ('full','partial'))
                AND NOT EXISTS (SELECT 1 FROM message_search_bodies b
                                 WHERE b.content_id = m.content_id)
              ORDER BY m.received_at DESC, m.id DESC
              LIMIT ?1",
            [limit],
            |row| {
                Ok(Candidate {
                    id: row.col::<i64>(0)?,
                    received_at: row.col::<i64>(1)?,
                })
            },
        )
        .await
        .map_err(Into::into),
        // Resumed at the cursor's timestamp on one column, not strictly after it on two:
        // `(received_at, id) < (?2, ?3)` scans and the OR form sorts the whole set;
        // only `received_at <= ?2` is a seek (measured on 40,000 messages). Dropping the `id`
        // tiebreaker is safe because indexing removes a message from this answer,
        // so `<=` repeats and skips nothing; the caller's last-batch guard catches a batch that
        // fails to index (#500).
        Some(cursor) => sql::all(
            connection,
            "SELECT m.id, m.received_at
               FROM messages m
              WHERE m.body_state IN ('full', 'partial')
                AND m.id = (SELECT min(id) FROM messages WHERE content_id = m.content_id AND body_state IN ('full','partial'))
                AND m.received_at <= ?2
                AND NOT EXISTS (SELECT 1 FROM message_search_bodies b
                                 WHERE b.content_id = m.content_id)
              ORDER BY m.received_at DESC, m.id DESC
              LIMIT ?1",
            bind![limit, cursor.received_at],
            |row| {
                Ok(Candidate {
                    id: row.col::<i64>(0)?,
                    received_at: row.col::<i64>(1)?,
                })
            },
        )
        .await
        .map_err(Into::into),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub id: i64,
    pub received_at: i64,
}

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
            AND m.id = (SELECT min(id) FROM messages WHERE content_id = m.content_id AND body_state IN ('full','partial'))
            AND NOT EXISTS (SELECT 1 FROM message_search_bodies b
                             WHERE b.content_id = m.content_id)
          ORDER BY m.received_at DESC
          LIMIT ?2",
        bind![account_id, limit],
        |row| row.col::<i64>(0),
    )
    .await
    .map_err(Into::into)
}

pub async fn clear_account_body_index(connection: &Connection, account_id: i64) -> Result<usize> {
    Ok(connection
        .execute(
            "DELETE FROM message_search_bodies
              WHERE content_id IN (SELECT id FROM message_contents WHERE account_id = ?1)",
            [account_id],
        )
        .await? as usize)
}

/// The only writer of `message_headers`, under ADR 0025 Q3's two caps. It normalizes rather than
/// trusting its caller: the index holds a 512-byte prefix
/// and the in-memory matcher whatever it was handed (#479), so both must pass through the same
/// function.
/// A message with no fields still gets one row with an empty name: the rows record that it was
/// indexed, or [`messages_missing_header_rows`] would offer it every lap (#500). The parser refuses
/// an empty header name, so it never matches.
pub async fn index_headers(
    connection: &Connection,
    message_id: i64,
    headers: &postio_model::Headers,
) -> Result<()> {
    let Some(content_id) = content_id_of(connection, message_id).await? else {
        return Ok(());
    };
    postio_storage::repository::MessageRepository::new(connection)
        .set_promoted(
            postio_model::MessageId::new(message_id),
            postio_model::promoted::PromotedHeaders::from_headers(headers),
        )
        .await?;
    // Delete first: re-indexing is the ordinary case, and an upsert would leave the rows of a
    // message that has lost a header.
    connection
        .execute(
            "DELETE FROM message_headers WHERE content_id = ?1",
            [content_id],
        )
        .await?;

    let normalized = headers.normalized();
    let mut statement = connection
        .prepare(
            "INSERT INTO message_headers (content_id, name, value, ordinal)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (content_id, name, ordinal) DO NOTHING",
        )
        .await?;
    let mut written = 0usize;
    for (ordinal, header) in normalized.iter().take(HEADER_ROWS_PER_MESSAGE).enumerate() {
        // A field whose name was only whitespace has none: it would collide with the "indexed,
        // nothing there" row below.
        if header.name.is_empty() {
            continue;
        }
        statement
            .execute(bind![content_id, header.name, header.value, ordinal as i64])
            .await?;
        written += 1;
    }
    if written == 0 {
        statement.execute(bind![content_id, "", "", 0i64]).await?;
    }
    Ok(())
}

/// Asked before the delete: the delete alone would succeed against no rows and the insert would
/// fail, turning an expunge race into an error.
async fn content_id_of(connection: &Connection, message_id: i64) -> Result<Option<i64>> {
    let found: Option<i64> = sql::first(
        connection,
        "SELECT content_id FROM messages WHERE id = ?1",
        [message_id],
        |row| row.col(0),
    )
    .await?;
    Ok(found)
}

/// Local only: `body_headers IS NOT NULL` is ADR 0025 Q5's population a pass with no network can
/// finish. A message whose block was never stored belongs to
/// `MessageRepository::messages_missing_headers` or `messages_needing_a_header_fetch`; mixing them
/// would give batches this pass can make no progress on.
pub async fn messages_missing_header_rows(connection: &Connection, limit: u32) -> Result<Vec<i64>> {
    sql::all(
        connection,
        "SELECT m.id
           FROM messages m
          WHERE m.body_has_headers = 1
            AND m.id = (SELECT min(id) FROM messages WHERE content_id = m.content_id)
            AND NOT EXISTS (SELECT 1 FROM message_headers h WHERE h.content_id = m.content_id)
          ORDER BY m.received_at DESC
          LIMIT ?1",
        [limit],
        |row| row.col::<i64>(0),
    )
    .await
    .map_err(Into::into)
}

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
            AND m.body_has_headers = 1
            AND m.id = (SELECT min(id) FROM messages WHERE content_id = m.content_id)
            AND NOT EXISTS (SELECT 1 FROM message_headers h WHERE h.content_id = m.content_id)
          ORDER BY m.received_at DESC
          LIMIT ?2",
        bind![account_id, limit],
        |row| row.col::<i64>(0),
    )
    .await
    .map_err(Into::into)
}

pub async fn clear_account_header_index(connection: &Connection, account_id: i64) -> Result<usize> {
    Ok(connection
        .execute(
            "DELETE FROM message_headers
          WHERE content_id IN (SELECT id FROM message_contents WHERE account_id = ?1)",
            [account_id],
        )
        .await? as usize)
}

/// For a sync batch, inside its transaction: every trigger write is an operation on
/// `search_documents_fts` and the engine's cost is per operation, so writing each message once with
/// [`write_documents`] is the only lever.
/// Only inside a transaction that ends in [`write_documents`]: the deferral is a row and outlives
/// the call, so left standing every later write would go unindexed; a rollback takes it away.
/// Answers whether there is an index to defer; a store [`ensure_schema`] has not reached has no
/// triggers.
pub async fn defer_documents(connection: &Connection) -> postio_storage::Result<bool> {
    let indexed = sql::exists(
        connection,
        "SELECT 1 FROM sqlite_schema
          WHERE type = 'table' AND name = 'search_documents_deferred'",
        (),
    )
    .await?;
    if indexed {
        sql::execute(
            connection,
            "INSERT INTO search_documents_deferred (deferred) VALUES (1)
             ON CONFLICT (deferred) DO NOTHING",
            (),
        )
        .await?;
    }
    Ok(indexed)
}

/// A document whose text has not changed is not written at all, so a resync of unchanged mail costs
/// nothing. The text is built as the triggers and the backfill in [`SCHEMA`] build it.
pub async fn write_documents(
    connection: &Connection,
    messages: &[postio_model::MessageId],
) -> postio_storage::Result<u64> {
    // One JSON-array placeholder whatever the batch size, so the statement compiles once and is
    // cached.
    let ids = format!(
        "[{}]",
        messages
            .iter()
            .map(|message| message.get().to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let written = sql::execute(
        connection,
        "INSERT INTO search_documents (content_id, subject, sender, recipients, filenames, list_id)
         SELECT
             m.content_id,
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
          WHERE m.id IN (SELECT value FROM json_each(?1))
          GROUP BY m.content_id
         ON CONFLICT (content_id) DO UPDATE SET
             subject = excluded.subject,
             sender = excluded.sender,
             recipients = excluded.recipients,
             filenames = excluded.filenames,
             list_id = excluded.list_id
          WHERE search_documents.subject IS NOT excluded.subject
             OR search_documents.sender IS NOT excluded.sender
             OR search_documents.recipients IS NOT excluded.recipients
             OR search_documents.filenames IS NOT excluded.filenames
             OR search_documents.list_id IS NOT excluded.list_id",
        [ids],
    )
    .await?;
    sql::execute(connection, "DELETE FROM search_documents_deferred", ()).await?;
    Ok(written)
}

/// Dropped and recreated: the index is an ordinary index on an ordinary table. Use after a bulk
/// import that bypassed the triggers.
/// It does not rebuild the body index, which [`messages_missing_body_text`] and the pass behind it
/// catch up.
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

const BODY_INDEX: &str = "CREATE INDEX IF NOT EXISTS messages_body_fts ON message_search_bodies USING fts (body_search);";

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS search_documents (
    content_id  INTEGER PRIMARY KEY REFERENCES message_contents(id) ON DELETE CASCADE,
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
-- `body_search` is `message_contents.body_text` folded for search. It sits in a
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
-- The index itself is `BODY_INDEX`, run after this: `index_bodies` drops
-- and builds it again on its own.

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
    content_id INTEGER NOT NULL REFERENCES message_contents(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,   -- lowercased; RFC 5322 names are case-insensitive
    value      TEXT    NOT NULL,   -- unfolded, RFC 2047-decoded, truncated at VALUE_LIMIT
    ordinal    INTEGER NOT NULL,   -- position within the message, wire order
    PRIMARY KEY (content_id, name, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_message_headers_name ON message_headers (name, content_id);

CREATE TRIGGER IF NOT EXISTS trg_message_contents_header_index_au
AFTER UPDATE OF body_headers ON message_contents
WHEN old.body_headers IS NOT new.body_headers
BEGIN
    DELETE FROM message_headers WHERE content_id = new.id;
END;

-- While this holds a row, every trigger below stands aside: a sync batch
-- is writing, and will write each message's document once, whole, when it
-- is done (`defer_documents`, `write_documents`). Each trigger write is an
-- operation on the full-text index -- ~2.3 ms apiece, measured -- and a
-- message with five addresses and an attachment was eight of them on insert,
-- and twice that on every resync, which deletes and rewrites its addresses
-- (#1587). The row lives only inside the batch's own transaction, so no
-- other writer ever sees it.
CREATE TABLE IF NOT EXISTS search_documents_deferred (
    deferred INTEGER PRIMARY KEY CHECK (deferred = 1)
);

CREATE TRIGGER IF NOT EXISTS trg_search_documents_messages_ai
AFTER INSERT ON messages
WHEN new.content_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM search_documents_deferred)
BEGIN
    INSERT INTO search_documents (content_id, subject, list_id)
    VALUES (new.content_id, coalesce(new.subject, ''), coalesce(new.list_id, ''))
    ON CONFLICT (content_id) DO UPDATE SET subject = excluded.subject, list_id = excluded.list_id;
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_messages_au
AFTER UPDATE OF subject, list_id, content_id ON messages
WHEN NOT EXISTS (SELECT 1 FROM search_documents_deferred)
BEGIN
    INSERT INTO search_documents (content_id, subject, list_id)
    SELECT new.content_id, coalesce(new.subject, ''), coalesce(new.list_id, '')
     WHERE new.content_id IS NOT NULL
    ON CONFLICT (content_id) DO UPDATE SET subject = excluded.subject, list_id = excluded.list_id
      WHERE subject IS NOT excluded.subject OR list_id IS NOT excluded.list_id;
END;

-- recipients -> search_documents: sender (kind = 'from') and recipients
-- (kind in to/cc/bcc), recomputed from scratch for the affected message(s)
-- on every change. A message has a handful of recipient rows at most, so
-- re-aggregating the lot is cheaper than tracking a per-kind delta.
CREATE TRIGGER IF NOT EXISTS trg_search_documents_recipients_ai
AFTER INSERT ON recipients
WHEN new.message_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM search_documents_deferred)
BEGIN
    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                  FROM recipients r JOIN addresses a ON a.id = r.address_id
                 WHERE r.message_id = new.message_id AND r.kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                      FROM recipients r JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = new.message_id AND r.kind IN ('to', 'cc', 'bcc'))
    WHERE content_id = (SELECT content_id FROM messages WHERE id = new.message_id);
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_recipients_ad
AFTER DELETE ON recipients
WHEN old.message_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM search_documents_deferred)
BEGIN
    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                  FROM recipients r JOIN addresses a ON a.id = r.address_id
                 WHERE r.message_id = old.message_id AND r.kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                      FROM recipients r JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = old.message_id AND r.kind IN ('to', 'cc', 'bcc'))
    WHERE content_id = (SELECT content_id FROM messages WHERE id = old.message_id);
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_recipients_au
AFTER UPDATE ON recipients
WHEN NOT EXISTS (SELECT 1 FROM search_documents_deferred)
BEGIN
    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                  FROM recipients r JOIN addresses a ON a.id = r.address_id
                 WHERE r.message_id = old.message_id AND r.kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                      FROM recipients r JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = old.message_id AND r.kind IN ('to', 'cc', 'bcc'))
    WHERE content_id = (SELECT content_id FROM messages WHERE id = old.message_id) AND old.message_id IS NOT NULL;

    UPDATE search_documents SET
        sender = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                  FROM recipients r JOIN addresses a ON a.id = r.address_id
                 WHERE r.message_id = new.message_id AND r.kind = 'from'),
        recipients = (SELECT coalesce(group_concat(coalesce(r.name, '') || ' ' || a.address, ' '), '')
                      FROM recipients r JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = new.message_id AND r.kind IN ('to', 'cc', 'bcc'))
    WHERE content_id = (SELECT content_id FROM messages WHERE id = new.message_id) AND new.message_id IS NOT NULL;
END;

-- attachments -> search_documents: filenames, same recompute-from-scratch
-- approach.
CREATE TRIGGER IF NOT EXISTS trg_search_documents_attachments_ai
AFTER INSERT ON attachments
WHEN new.message_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM search_documents_deferred)
BEGIN
    UPDATE search_documents SET
        filenames = (SELECT coalesce(group_concat(filename, ' '), '')
                     FROM attachments WHERE message_id = new.message_id AND filename IS NOT NULL)
    WHERE content_id = (SELECT content_id FROM messages WHERE id = new.message_id);
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_attachments_ad
AFTER DELETE ON attachments
WHEN old.message_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM search_documents_deferred)
BEGIN
    UPDATE search_documents SET
        filenames = (SELECT coalesce(group_concat(filename, ' '), '')
                     FROM attachments WHERE message_id = old.message_id AND filename IS NOT NULL)
    WHERE content_id = (SELECT content_id FROM messages WHERE id = old.message_id);
END;

CREATE TRIGGER IF NOT EXISTS trg_search_documents_attachments_au
AFTER UPDATE OF filename, message_id ON attachments
WHEN NOT EXISTS (SELECT 1 FROM search_documents_deferred)
BEGIN
    UPDATE search_documents SET
        filenames = (SELECT coalesce(group_concat(filename, ' '), '')
                     FROM attachments WHERE message_id = old.message_id AND filename IS NOT NULL)
    WHERE content_id = (SELECT content_id FROM messages WHERE id = old.message_id) AND old.message_id IS NOT NULL;

    UPDATE search_documents SET
        filenames = (SELECT coalesce(group_concat(filename, ' '), '')
                     FROM attachments WHERE message_id = new.message_id AND filename IS NOT NULL)
    WHERE content_id = (SELECT content_id FROM messages WHERE id = new.message_id) AND new.message_id IS NOT NULL;
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
INSERT INTO search_documents (content_id, subject, sender, recipients, filenames, list_id)
SELECT
    m.content_id,
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
WHERE m.id = (SELECT min(id) FROM messages WHERE content_id = m.content_id)
ON CONFLICT (content_id) DO NOTHING;
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingAttachment {
    /// Any occurrence will do: [`index_attachment_text`] keys the text by content and position.
    pub attachment: postio_model::AttachmentId,
    pub message: postio_model::MessageId,
    pub blob: postio_model::BlobId,
    pub mime_type: String,
    pub name: Option<String>,
}

pub(crate) const MISSING: &str =
    "SELECT a.id, a.message_id, a.blob_id, a.mime_type, a.filename, m.content_id, a.position
  FROM attachments a JOIN messages m ON m.id = a.message_id
 WHERE a.blob_id IS NOT NULL AND m.content_id IS NOT NULL
   AND NOT EXISTS (SELECT 1 FROM attachment_extraction e
                    WHERE e.content_id = m.content_id AND e.position = a.position
                      AND e.version = ?1)";

fn missing_row(row: &turso::Row) -> postio_storage::Result<(MissingAttachment, (i64, i64))> {
    Ok((
        MissingAttachment {
            attachment: postio_model::AttachmentId::new(row.col(0)?),
            message: postio_model::MessageId::new(row.col(1)?),
            blob: postio_model::BlobId::new(row.col::<String>(2)?),
            mime_type: row.col(3)?,
            name: row.col(4)?,
        },
        (row.col(5)?, row.col(6)?),
    ))
}

fn first_of_each(rows: Vec<(MissingAttachment, (i64, i64))>) -> Vec<MissingAttachment> {
    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|(_, key)| seen.insert(*key))
        .map(|(missing, _)| missing)
        .collect()
}

/// Only attachments whose blob is on this machine; nothing asks for the bytes of one never
/// downloaded. A part that was tried has a row whatever came of it,
/// so the queue never offers it twice (#500); one from an older extractor is missing again, which
/// is how a bump re-extracts.
pub async fn attachments_missing_text(
    connection: &Connection,
    limit: u32,
) -> Result<Vec<MissingAttachment>> {
    // Twice the limit, then one per payload part: a page of duplicates still leaves something to
    // do.
    let rows = sql::all(
        connection,
        &format!("{MISSING} ORDER BY a.id DESC LIMIT ?2"),
        (
            i64::from(postio_extract::EXTRACTOR_VERSION),
            i64::from(limit).saturating_mul(2),
        ),
        missing_row,
    )
    .await?;
    let mut missing = first_of_each(rows);
    missing.truncate(limit as usize);
    Ok(missing)
}

pub async fn attachments_missing_text_of(
    connection: &Connection,
    messages: &[postio_model::MessageId],
) -> Result<Vec<MissingAttachment>> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let ids = format!(
        "[{}]",
        messages
            .iter()
            .map(|message| message.get().to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let rows = sql::all_unbounded(
        connection,
        &format!("{MISSING} AND a.message_id IN (SELECT value FROM json_each(?2)) ORDER BY a.id"),
        (i64::from(postio_extract::EXTRACTOR_VERSION), ids),
        missing_row,
    )
    .await?;
    Ok(first_of_each(rows))
}

/// An extraction with no units still records its outcome: that row takes the part off the queue.
/// `false` and nothing written when the attachment is gone
/// (expunged, or a refetch replaced its row), which is not an error. Run it inside the caller's
/// transaction.
pub async fn index_attachment_text(
    connection: &Connection,
    attachment: postio_model::AttachmentId,
    extracted: &postio_extract::Extracted,
) -> Result<bool> {
    let Some((content_id, position)) = part_of(connection, attachment).await? else {
        return Ok(false);
    };
    connection
        .execute(
            "DELETE FROM attachment_passages WHERE content_id = ?1 AND position = ?2",
            (content_id, position),
        )
        .await?;
    if !extracted.units.is_empty() {
        let mut statement = connection
            .prepare(
                "INSERT INTO attachment_passages
                     (content_id, position, ordinal, location, text, text_search)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .await?;
        for (ordinal, unit) in extracted.units.iter().enumerate() {
            statement
                .execute(bind![
                    content_id,
                    position,
                    ordinal as i64,
                    encode_location(&unit.location),
                    unit.text.as_str(),
                    postio_model::fold::fold(&unit.text)
                ])
                .await?;
        }
    }
    connection
        .execute(
            "INSERT INTO attachment_extraction (content_id, position, version, outcome, units)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (content_id, position) DO UPDATE SET
                 version = excluded.version,
                 outcome = excluded.outcome,
                 units = excluded.units",
            bind![
                content_id,
                position,
                i64::from(postio_extract::EXTRACTOR_VERSION),
                recorded(extracted.outcome),
                extracted.units.len() as i64
            ],
        )
        .await?;
    Ok(true)
}

/// A part skipped for want of its reader is `unavailable`, not `skipped`, because that skip is not
/// final ([`requeue_unavailable`]).
fn recorded(outcome: postio_extract::Outcome) -> &'static str {
    match outcome {
        postio_extract::Outcome::Skipped(postio_extract::Skip::Unavailable) => "unavailable",
        other => other.as_str(),
    }
}

/// The row is what kept the part off the queue so a pass with no helper ends (#500). Called by the
/// indexer before a pass when it can see the helper, not by the queue:
/// a queue that offered these parts without the helper would offer them for ever.
pub async fn requeue_unavailable(connection: &Connection) -> Result<u64> {
    Ok(sql::execute(
        connection,
        "DELETE FROM attachment_extraction WHERE outcome = 'unavailable'",
        (),
    )
    .await?)
}

async fn part_of(
    connection: &Connection,
    attachment: postio_model::AttachmentId,
) -> Result<Option<(i64, i64)>> {
    Ok(sql::first(
        connection,
        "SELECT m.content_id, a.position FROM attachments a JOIN messages m ON m.id = a.message_id
          WHERE a.id = ?1 AND m.content_id IS NOT NULL",
        [attachment.get()],
        |row| Ok((row.col(0)?, row.col(1)?)),
    )
    .await?)
}

pub async fn attachment_text(
    connection: &Connection,
    attachment: postio_model::AttachmentId,
) -> Result<Vec<(postio_search::results::Location, String)>> {
    let Some((content_id, position)) = part_of(connection, attachment).await? else {
        return Ok(Vec::new());
    };
    let rows: Vec<(String, String)> = sql::all_unbounded(
        connection,
        "SELECT location, text FROM attachment_passages
          WHERE content_id = ?1 AND position = ?2 ORDER BY ordinal",
        (content_id, position),
        |row| Ok((row.col(0)?, row.col(1)?)),
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(location, text)| decode_location(&location).map(|location| (location, text)))
        .collect())
}

/// A unit that is not there (the message went, the extractor changed) is left out.
pub async fn attachment_units(
    connection: &Connection,
    wanted: &[(postio_model::AttachmentId, postio_search::results::Location)],
) -> Result<
    Vec<(
        postio_model::AttachmentId,
        postio_search::results::Location,
        String,
    )>,
> {
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    // JSON, so a sheet's name is quoted whatever it holds.
    let pairs = format!(
        "[{}]",
        wanted
            .iter()
            .map(|(attachment, location)| format!(
                "[{},{}]",
                attachment.get(),
                json_string(&encode_location(location))
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let rows: Vec<(i64, String, String)> = sql::all_unbounded(
        connection,
        "SELECT a.id, p.location, p.text
           FROM json_each(?1) j
           JOIN attachments a ON a.id = json_extract(j.value, '$[0]')
           JOIN messages m ON m.id = a.message_id
           JOIN attachment_passages p
             ON p.content_id = m.content_id AND p.position = a.position
            AND p.location = json_extract(j.value, '$[1]')",
        [pairs],
        |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(attachment, location, text)| {
            Some((
                postio_model::AttachmentId::new(attachment),
                decode_location(&location)?,
                text,
            ))
        })
        .collect())
}

fn json_string(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            c if u32::from(c) < 0x20 => quoted.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// A sheet's name may hold a colon, so its row is read from the right.
pub fn encode_location(location: &postio_search::results::Location) -> String {
    use postio_search::results::Location;
    match location {
        Location::Page(n) => format!("page:{n}"),
        Location::Sheet { name, row } => format!("sheet:{name}:{row}"),
        Location::Slide(n) => format!("slide:{n}"),
        Location::Paragraph(n) => format!("paragraph:{n}"),
        Location::Line(n) => format!("line:{n}"),
        Location::Table { index, row } => format!("table:{index}:{row}"),
        Location::ImageText => "image".to_owned(),
    }
}

pub fn decode_location(encoded: &str) -> Option<postio_search::results::Location> {
    use postio_search::results::Location;
    if encoded == "image" {
        return Some(Location::ImageText);
    }
    let (kind, rest) = encoded.split_once(':')?;
    match kind {
        "page" => rest.parse().ok().map(Location::Page),
        "slide" => rest.parse().ok().map(Location::Slide),
        "paragraph" => rest.parse().ok().map(Location::Paragraph),
        "line" => rest.parse().ok().map(Location::Line),
        "sheet" => {
            let (name, row) = rest.rsplit_once(':')?;
            Some(Location::Sheet {
                name: name.to_owned(),
                row: row.parse().ok()?,
            })
        }
        "table" => {
            let (index, row) = rest.split_once(':')?;
            Some(Location::Table {
                index: index.parse().ok()?,
                row: row.parse().ok()?,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use postio_model::{Attachment, EmailAddress, Message};
    use postio_storage::repository::MessageRepository;
    use postio_storage::test_support;

    async fn matches(connection: &Connection, query: &str) -> Vec<i64> {
        sql::all(
            connection,
            "SELECT content_id FROM search_documents
              WHERE fts_match(sender, recipients, subject, filenames, list_id, ?1)
              ORDER BY content_id",
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

    async fn document(
        connection: &Connection,
        message: i64,
    ) -> Option<(String, String, String, String)> {
        sql::first(
            connection,
            "SELECT subject, sender, recipients, filenames FROM search_documents
              WHERE content_id = (SELECT content_id FROM messages WHERE id = ?1)",
            [message],
            |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?, row.col(3)?)),
        )
        .await
        .expect("a read")
    }

    fn addressed(
        account: &postio_model::Account,
        mailbox: postio_model::MailboxId,
        n: u32,
    ) -> Message {
        let mut message = Message::new(account.id, mailbox, Utc::now());
        message.subject = Some(format!("Harbour schedule {n}"));
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.to = vec![
            EmailAddress::new(Some("Bo"), "bo@example.org"),
            EmailAddress::new(None::<String>, "cy@example.net"),
        ];
        message.cc = vec![EmailAddress::new(Some("Di"), "di@example.test")];
        let mut attachment = Attachment::new(message.id, "application/pdf", 1024);
        attachment.filename = Some(format!("tide-table-{n}.pdf"));
        message.attachments = vec![attachment];
        message.server.uid = Some(postio_model::Uid::new(n));
        message.server.uid_validity = Some(postio_model::UidValidity::new(1));
        message
    }

    #[tokio::test]
    async fn a_deferred_batch_is_indexed_once_per_message_and_completely() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;
        let mut batch: Vec<Message> = (1..=3).map(|n| addressed(&account, mailbox, n)).collect();

        defer_documents(&connection).await.expect("defer");
        MessageRepository::new(&connection)
            .upsert_batch(&mut batch)
            .await
            .expect("the batch");
        assert_eq!(
            document(&connection, batch[0].id.get()).await,
            None,
            "deferred: the triggers wrote nothing"
        );

        let ids: Vec<_> = batch.iter().map(|message| message.id).collect();
        let written = write_documents(&connection, &ids).await.expect("documents");

        assert_eq!(written, 3, "one document per message");
        let (subject, sender, recipients, filenames) = document(&connection, batch[1].id.get())
            .await
            .expect("a document");
        assert_eq!(subject, "Harbour schedule 2");
        assert!(sender.contains("Ada Lovelace") && sender.contains("ada@example.com"));
        assert!(recipients.contains("bo@example.org") && recipients.contains("di@example.test"));
        assert_eq!(filenames, "tide-table-2.pdf");
        assert_eq!(
            matches(&connection, "tide").await.len(),
            3,
            "and searchable"
        );

        let mut later = Message::new(account.id, mailbox, Utc::now());
        later.subject = Some("Lighthouse".to_owned());
        MessageRepository::new(&connection)
            .create(&mut later)
            .await
            .expect("create");
        assert_eq!(
            matches(&connection, "lighthouse").await,
            vec![later.id.get()]
        );
    }

    #[tokio::test]
    async fn resyncing_unchanged_mail_writes_no_document() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        let mut batch: Vec<Message> = (1..=3).map(|n| addressed(&account, mailbox, n)).collect();
        defer_documents(&connection).await.expect("defer");
        repository
            .upsert_batch(&mut batch)
            .await
            .expect("first pass");
        let ids: Vec<_> = batch.iter().map(|message| message.id).collect();
        write_documents(&connection, &ids).await.expect("documents");

        let mut again: Vec<Message> = (1..=3).map(|n| addressed(&account, mailbox, n)).collect();
        again[2].subject = Some("Harbour schedule, revised".to_owned());
        defer_documents(&connection).await.expect("defer");
        repository
            .upsert_batch(&mut again)
            .await
            .expect("second pass");
        let written = write_documents(&connection, &ids).await.expect("documents");

        assert_eq!(written, 1, "only the message whose subject moved");
        assert_eq!(matches(&connection, "revised").await, vec![ids[2].get()]);
        assert_eq!(
            matches(&connection, "ada").await.len(),
            3,
            "the rest unchanged"
        );
    }

    async fn body_matches(connection: &Connection, query: &str) -> Vec<i64> {
        let folded = postio_model::fold::fold(query);
        sql::all(
            connection,
            "SELECT content_id FROM message_search_bodies              WHERE fts_match(body_search, ?1) ORDER BY content_id",
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
    async fn a_sweep_resumes_after_its_cursor_rather_than_restarting() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;

        let messages = MessageRepository::new(&connection);
        let mut created = Vec::new();
        for minute in 0..6 {
            let mut message = Message::new(
                account.id,
                inbox,
                Utc::now() - chrono::Duration::minutes(minute),
            );
            message.sync.body_state = postio_model::BodyState::Full;
            messages.create(&mut message).await.expect("create");
            created.push(message.id.get());
        }

        let mut swept = Vec::new();
        let mut cursor = None;
        loop {
            let batch = messages_missing_body_text(&connection, 2, cursor)
                .await
                .expect("a batch");
            if batch.is_empty() {
                break;
            }
            assert!(batch.len() <= 2, "the limit is honoured");
            cursor = batch.last().copied();
            for candidate in &batch {
                swept.push(candidate.id);
                index_body_of(
                    &connection,
                    candidate.id,
                    &MessageBody {
                        text: Some("body".to_owned()),
                        html: None,
                    },
                )
                .await
                .expect("index");
            }
        }

        assert_eq!(
            swept, created,
            "every message exactly once, newest first, across three resumed batches"
        );
    }

    #[tokio::test]
    async fn an_indexed_message_is_not_a_candidate_after_a_cursor_either() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        ensure_schema(&connection).await.expect("schema");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;

        let messages = MessageRepository::new(&connection);
        let mut created = Vec::new();
        for minute in 0..3 {
            let mut message = Message::new(
                account.id,
                inbox,
                Utc::now() - chrono::Duration::minutes(minute),
            );
            message.sync.body_state = postio_model::BodyState::Full;
            messages.create(&mut message).await.expect("create");
            created.push(message.id.get());
        }

        index_body_of(
            &connection,
            created[1],
            &MessageBody {
                text: Some("already done".to_owned()),
                html: None,
            },
        )
        .await
        .expect("index the middle");

        let first = messages_missing_body_text(&connection, 1, None)
            .await
            .expect("first batch");
        assert_eq!(first[0].id, created[0], "the newest, and it is not indexed");

        index_body_of(
            &connection,
            first[0].id,
            &MessageBody {
                text: Some("just done".to_owned()),
                html: None,
            },
        )
        .await
        .expect("index the first");

        let rest = messages_missing_body_text(&connection, 10, first.last().copied())
            .await
            .expect("the rest");
        assert_eq!(
            rest.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![created[2]],
            "neither the one just indexed nor the one already indexed comes back"
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
