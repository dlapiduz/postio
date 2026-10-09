//! What attachments say, as a search finds it (spec 010 step 9, US8).
//!
//! The conversation search's third arm ([`super::HITS_JOIN_WITH_FILES`])
//! says only *that* a message matched in an attachment. Which attachment,
//! and where in it, is read here, for the few messages that are shown: the
//! rows of a page, the messages of the conversation Quick Look is open on.

use postio_model::{AccountScope, AttachmentId, MessageId};
use postio_search::ParsedQuery;
use postio_search::facets::Scope;
use postio_search::results::Location;
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _};

use super::{Plan, SearchRequest};
use crate::error::Result;

/// The first unit of one attachment that a query's words match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatch {
    /// The message carrying it.
    pub message: MessageId,
    /// The attachment.
    pub attachment: AttachmentId,
    /// Its file name; empty when it has none.
    pub name: String,
    /// Where in it: a page, a sheet's row, a slide.
    pub location: Location,
    /// That unit's text as extracted: what a passage is cut from.
    pub text: String,
}

/// The words of `query` as the attachment index matches them: folded, as
/// the body's half is, since its text is folded the same way. `None` when
/// the query has no positive words.
fn folded_match(query: &ParsedQuery) -> Option<turso::Value> {
    Plan::build(&SearchRequest {
        account: AccountScope::Unified,
        query,
        scope: Scope::AllMail,
        limit: 0,
        order: postio_search::ResultOrder::Relevance,
    })
    .body_match_param
}

/// For each attachment of `messages` whose text `query`'s words match, its
/// first matching unit, in the order of the messages' ids, then the
/// attachments' positions. One statement; nothing when the query has no
/// words.
///
/// The match is read in a subquery and joined out, as `HITS_JOIN` reads its
/// hits: the index answers the match, and the join keeps the messages
/// asked about.
pub async fn file_matches(
    connection: &Connection,
    query: &ParsedQuery,
    messages: &[MessageId],
) -> Result<Vec<FileMatch>> {
    let Some(words) = folded_match(query) else {
        return Ok(Vec::new());
    };
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let ids = format!(
        "[{}]",
        messages
            .iter()
            .map(|id| id.get().to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let rows: Vec<(i64, i64, String, String, String)> = sql::all_unbounded(
        connection,
        "SELECT m.id, a.id, coalesce(a.filename, ''), p.location, p.text
           FROM (SELECT content_id, position, ordinal, location, text
                   FROM attachment_passages WHERE fts_match(text_search, ?1)) p
           CROSS JOIN messages m ON m.content_id = p.content_id
           JOIN attachments a ON a.message_id = m.id AND a.position = p.position
          WHERE m.id IN (SELECT value FROM json_each(?2))
          ORDER BY m.id, a.position, p.ordinal",
        (words, ids),
        |row| {
            Ok((
                row.col(0)?,
                row.col(1)?,
                row.col(2)?,
                row.col(3)?,
                row.col(4)?,
            ))
        },
    )
    .await?;
    let mut found: Vec<FileMatch> = Vec::new();
    for (message, attachment, name, location, text) in rows {
        let attachment = AttachmentId::new(attachment);
        if found.iter().any(|each| each.attachment == attachment) {
            continue;
        }
        let Some(location) = crate::index::decode_location(&location) else {
            continue;
        };
        found.push(FileMatch {
            message: MessageId::new(message),
            attachment,
            name,
            location,
            text,
        });
    }
    Ok(found)
}

/// How many cards the Files tab can hold: the walk is capped as a
/// conversation search's is, and this caps what one answer carries.
pub const FILES_CAP: u32 = 1_000;

/// The Files tab (design §3.8): one card per attachment of the mail the
/// query matches whose name or contents match its words -- or, for a
/// query with no words, every attachment of that mail -- newest first,
/// each with where it matched and the words around it.
///
/// Three statements: the walk, the attachments of the mail it found, and
/// where in the files their text matched (none when nothing matched in a
/// file). An attachment whose bytes were never downloaded can still match
/// by its name; nothing is fetched to look inside it (FR-050).
pub async fn files(
    connection: &Connection,
    request: &super::ConversationRequest<'_>,
) -> Result<Vec<postio_search::results::FileHit>> {
    use postio_search::passage::FirstLine;
    use postio_search::results::{FileHit, Match, Source};

    let carriers = super::conversations::carriers(connection, request).await?;
    if carriers.is_empty() {
        return Ok(Vec::new());
    }
    let ids = format!(
        "[{}]",
        carriers
            .iter()
            .map(|carrier| carrier.id.get().to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    // Parts that are shown in the body (a logo by its `cid:`) are not
    // files anyone sent.
    let rows: Vec<Row> = sql::all_unbounded(
        connection,
        "SELECT f.id, f.message_id, coalesce(f.filename, ''), f.mime_type, f.size,
                sub.subject, sub.received_at, sender.name, a.address
           FROM json_each(?1) j
           JOIN attachments f ON f.message_id = j.value
           JOIN (SELECT m.id, m.subject, m.received_at,
                        (SELECT r.id FROM recipients r
                          WHERE r.message_id = m.id AND r.kind = 'from'
                          ORDER BY r.position LIMIT 1) AS from_recipient
                   FROM messages m
                  WHERE m.id IN (SELECT value FROM json_each(?1))) sub ON sub.id = f.message_id
           LEFT JOIN recipients sender ON sender.id = sub.from_recipient
           LEFT JOIN addresses a ON a.id = sender.address_id
          WHERE NOT (f.disposition = 'inline' AND f.content_id IS NOT NULL)",
        [ids.as_str()],
        |row| {
            Ok(Row {
                attachment: row.col(0)?,
                message: row.col(1)?,
                name: row.col(2)?,
                mime_type: row.col(3)?,
                size: row.col(4)?,
                subject: row.col(5)?,
                received_at: row.col(6)?,
                from_name: row.col(7)?,
                from_address: row.col(8)?,
            })
        },
    )
    .await?;

    let in_files: Vec<MessageId> = carriers
        .iter()
        .filter(|carrier| carrier.in_file)
        .map(|carrier| carrier.id)
        .collect();
    let read = if in_files.is_empty() {
        Vec::new()
    } else {
        file_matches(connection, request.query, &in_files).await?
    };
    // A name is checked for the words and `filename:`'s values; a unit's
    // text for the words, as the index matched it.
    let name_terms = super::conversations::file_terms(request.query);
    let words: Vec<String> = request
        .query
        .searchable_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.clone())
        .filter(|value| !value.is_empty())
        .collect();

    let mut cards: Vec<FileHit> = Vec::new();
    for row in rows {
        let attachment = AttachmentId::new(row.attachment);
        let when = postio_storage::repository::from_millis(row.received_at);
        let inside = read.iter().find(|file| file.attachment == attachment);
        let named = postio_search::highlight::find(&row.name, &name_terms);
        let matched = match inside {
            Some(file) => Some(Match {
                source: Source::FileContent {
                    attachment,
                    name: row.name.clone(),
                    location: file.location.clone(),
                },
                passage: postio_search::passage::cut(&file.text, &words, FirstLine::Any),
                when: Some(when),
            }),
            None if !named.is_empty() => Some(Match {
                source: Source::FileName {
                    attachment,
                    name: row.name.clone(),
                },
                passage: Some(postio_search::passage::Passage {
                    text: row.name.clone(),
                    ranges: named,
                    elided_start: false,
                    elided_end: false,
                }),
                when: Some(when),
            }),
            // With words, a file that matched neither way is in matching
            // mail but is not itself a match.
            None if !name_terms.is_empty() => continue,
            None => None,
        };
        cards.push(FileHit {
            attachment,
            message: MessageId::new(row.message),
            name: row.name,
            mime_type: row.mime_type,
            size: row.size.max(0) as u64,
            from: row
                .from_address
                .map(|address| postio_model::EmailAddress::new(row.from_name, address)),
            received_at: when,
            subject: row.subject,
            matched,
        });
    }
    cards.sort_by(|a, b| {
        b.received_at
            .cmp(&a.received_at)
            .then(b.message.cmp(&a.message))
            .then(a.attachment.cmp(&b.attachment))
    });
    let offset = (request.offset as usize).min(cards.len());
    let end = offset
        .saturating_add(request.limit.min(FILES_CAP) as usize)
        .min(cards.len());
    Ok(cards.drain(offset..end).collect())
}

/// One attachment row, as [`files`] reads it.
struct Row {
    attachment: i64,
    message: i64,
    name: String,
    mime_type: String,
    size: i64,
    subject: Option<String>,
    received_at: i64,
    from_name: Option<String>,
    from_address: Option<String>,
}
