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
