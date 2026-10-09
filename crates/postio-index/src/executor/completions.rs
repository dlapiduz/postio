//! What a prefix could become (spec 010, US7, FR-042): the dropdown's
//! short-prefix and operator states.
//!
//! The index has no term dictionary (research R1), so the words come the
//! way [`super::words_near`] recovers them: the best documents of a
//! `prefix*` match are read and split as the tokenizer splits. Metadata
//! first -- senders, subjects, file names and lists are what a person types
//! the beginning of -- and the bodies only when the metadata says fewer
//! than [`ENOUGH_WORDS`] (D22). A vocabulary table kept on every write is
//! the fallback if this misses its 20 ms, and only then.
//!
//! Every suggestion's count is what its query would find (US7's
//! independent test): the conversations [`super::relaxations`] counts, all
//! of them in one statement -- up to [`COMPLETION_COUNT_CAP`], past which
//! it is a floor (D29). So the whole answer is at most four
//! statements: the documents, the bodies when needed, the labels, the
//! counts.
//!
//! People for `from:` and `to:` are ranked two-way (D21): how often they
//! wrote to you (`contacts.times_seen`) plus how often you wrote to them
//! (`correspondents.sent_count`), the same sum a conversation's
//! `FrequentSender` reason is read from.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use postio_model::AccountScope;
use postio_search::query::{Clause, Field, Filter, spell};
use postio_search::suggest::{Completion, Person, Suggestions, Term, rank_words};
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _};

use super::escape_like;
use super::relaxations::counts;
use crate::error::Result;

/// How many documents of each half are read for words.
///
/// The same bound a suggestion for a search that found nothing reads
/// (`SUGGESTION_DOCUMENTS`): the index has already narrowed to documents
/// holding a word that begins so, and ranking needs relative counts.
const DOCUMENTS: i64 = 50;

/// Fewer words than this from the metadata, and the bodies are read too.
const ENOUGH_WORDS: usize = 3;

/// The completed words offered: the dropdown draws one, "as a word".
const WORDS: usize = 1;

/// Labels, folders and lists offered.
const LABELS: usize = 3;
const FOLDERS: usize = 4;
const LISTS: usize = 1;

/// File names kept: the row names two and says how many more.
const FILES: usize = 20;

/// People offered.
const PEOPLE: usize = 4;

/// Where a suggestion's count stops (D29, maintainer 2026-10-09): the
/// matches walked, as
/// [`CONVERSATION_WALK_CAP`](postio_search::results::CONVERSATION_WALK_CAP)
/// is the conversation search's. Past it the count is a floor -- the
/// conversations among the matches walked -- and the row says "N+".
///
/// Below it the count is exact, what the suggestion's query would find
/// (US7). A completion as common as `as`, in every message, walked the
/// whole mailbox to the search's own cap and cost 28 ms of a 20 ms
/// keystroke budget. On the step-1 corpus a thousand puts `a` at 4.4 ms at
/// p95, 2,500 at 8.2 and 6,500 (what a floor of a thousand *conversations*
/// would need there) at 18.1 (step-8 note, "Capped").
pub const COMPLETION_COUNT_CAP: u64 = 1_000;

/// The separator between a document's file names: a unit separator, which
/// no file name carries.
const UNIT: char = '\u{1f}';

/// The day the counted queries are parsed against. They name no dates --
/// a word, `label:`, `list:`, `in:`, `filename:` -- so any day reads them
/// the same.
fn undated() -> NaiveDate {
    NaiveDate::default()
}

/// What `prefix` could become: the words, labels, lists and file names it
/// begins, with no `field`; the people, labels or folders an operator's
/// value begins with `field`. Each is counted as its query would count.
pub async fn completions(
    connection: &Connection,
    account: AccountScope,
    prefix: &str,
    field: Option<Field>,
) -> Result<Suggestions> {
    let prefix = prefix.trim();
    match field {
        None => words_labels_lists_files(connection, account, prefix).await,
        Some(Field::From | Field::To) => Ok(Suggestions {
            people: people(connection, account, prefix).await?,
            ..Suggestions::default()
        }),
        Some(Field::Label) => {
            let names = label_names(connection, account, prefix).await?;
            let mut labels = completed(
                names
                    .into_iter()
                    .map(|name| (name.clone(), Filter::Label(name))),
            );
            count_all(connection, account, &mut [&mut labels]).await?;
            keep_found(&mut labels, LABELS);
            Ok(Suggestions {
                labels,
                ..Suggestions::default()
            })
        }
        Some(Field::In) => {
            let names = folder_names(connection, account, prefix).await?;
            let mut folders = completed(
                names
                    .into_iter()
                    .map(|name| (name.clone(), Filter::In(name))),
            );
            count_all(connection, account, &mut [&mut folders]).await?;
            folders.truncate(FOLDERS);
            Ok(Suggestions {
                folders,
                ..Suggestions::default()
            })
        }
        Some(_) => Ok(Suggestions::default()),
    }
}

/// The short-prefix state's suggestions.
async fn words_labels_lists_files(
    connection: &Connection,
    account: AccountScope,
    prefix: &str,
) -> Result<Suggestions> {
    let word = prefix.to_lowercase();
    let searchable = !word.is_empty() && word.chars().all(char::is_alphanumeric);
    let mut vocabulary: HashMap<String, u64> = HashMap::new();
    let mut lists: Vec<(String, u64)> = Vec::new();
    let mut files: Vec<(String, String, u64)> = Vec::new();
    if searchable {
        let documents = metadata(connection, &word).await?;
        for document in &documents {
            count_words(&mut vocabulary, document.texts.iter().flatten());
            if let Some(list) = document.list.as_deref().filter(|list| !list.is_empty())
                && begins_a_word(list, &word).is_some()
            {
                match lists.iter_mut().find(|(held, _)| held == list) {
                    Some((_, seen)) => *seen += 1,
                    None => lists.push((list.to_owned(), 1)),
                }
            }
            for name in &document.files {
                let Some(token) = begins_a_word(name, &word) else {
                    continue;
                };
                match files.iter_mut().find(|(held, _, _)| held == name) {
                    Some((_, _, seen)) => *seen += 1,
                    None => files.push((name.clone(), token, 1)),
                }
            }
        }
        let completing = vocabulary
            .keys()
            .filter(|held| held.len() > word.len() && held.starts_with(&word))
            .count();
        if completing < ENOUGH_WORDS {
            let bodies = bodies(connection, &word).await?;
            count_words(&mut vocabulary, bodies.iter());
        }
    }

    let mut words = rank_words(
        &word,
        vocabulary.iter().map(|(text, documents)| Term {
            text,
            documents: *documents,
        }),
    );
    words.truncate(WORDS);
    let mut labels = completed(
        label_names(connection, account, prefix)
            .await?
            .into_iter()
            .map(|name| (name.clone(), Filter::Label(name))),
    );
    lists.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut lists = completed(
        lists
            .into_iter()
            .map(|(list, _)| (list.clone(), Filter::List(list))),
    );
    lists.truncate(LISTS);

    count_all(
        connection,
        account,
        &mut [&mut words, &mut labels, &mut lists],
    )
    .await?;
    keep_found(&mut words, WORDS);
    keep_found(&mut labels, LABELS);
    keep_found(&mut lists, LISTS);

    files.truncate(FILES);
    let files = files
        .into_iter()
        .map(|(name, token, seen)| Completion {
            text: name,
            query: spell(&Clause {
                negated: false,
                filter: Filter::Filename(token),
            }),
            count: seen,
            capped: false,
        })
        .collect();

    let ghost = words
        .first()
        .and_then(|first| postio_search::suggest::ghost(prefix, &first.text));
    Ok(Suggestions {
        ghost,
        words,
        labels,
        lists,
        files,
        folders: Vec::new(),
        people: Vec::new(),
    })
}

/// One metadata document as read for words.
struct Document {
    /// Sender, recipients, subject, list.
    texts: Vec<Option<String>>,
    list: Option<String>,
    /// Its attachments' names, whole: the index's own column joins them
    /// with spaces, which a name may hold.
    files: Vec<String>,
}

/// The best [`DOCUMENTS`] metadata documents holding a word that begins
/// with `word`. One statement. `fts_score` is projected bare and with the
/// match's own parameter, or it answers 0.0 (see `HITS_JOIN`); the file
/// names are read for those documents only, outside the ordered match.
async fn metadata(connection: &Connection, word: &str) -> Result<Vec<Document>> {
    let documents = sql::all(
        connection,
        "SELECT d.sender, d.recipients, d.subject, d.list_id,
                (SELECT group_concat(f.filename, char(31))
                   FROM messages m JOIN attachments f ON f.message_id = m.id
                  WHERE m.content_id = d.content_id AND f.filename IS NOT NULL)
           FROM (SELECT content_id, sender, recipients, subject, list_id,
                        fts_score(sender, recipients, subject, filenames, list_id, ?1) AS score
                   FROM search_documents
                  WHERE fts_match(sender, recipients, subject, filenames, list_id, ?1)
                  ORDER BY score DESC
                  LIMIT ?2) d",
        (format!("{word}*"), DOCUMENTS),
        |row| {
            let list = row.opt_text(3)?;
            let mut files: Vec<String> = row
                .opt_text(4)?
                .map(|joined| joined.split(UNIT).map(str::to_owned).collect())
                .unwrap_or_default();
            files.dedup();
            Ok(Document {
                texts: vec![
                    row.opt_text(0)?,
                    row.opt_text(1)?,
                    row.opt_text(2)?,
                    list.clone(),
                    Some(files.join(" ")),
                ],
                list,
                files,
            })
        },
    )
    .await?;
    Ok(documents)
}

/// The best [`DOCUMENTS`] bodies holding a word that begins with `word`,
/// folded as the body index folds (ADR 0038). One statement.
async fn bodies(connection: &Connection, word: &str) -> Result<Vec<String>> {
    let folded = postio_model::fold::fold(word);
    let texts = sql::all(
        connection,
        "SELECT body_search, fts_score(body_search, ?1) AS score
           FROM message_search_bodies
          WHERE fts_match(body_search, ?1)
          ORDER BY score DESC
          LIMIT ?2",
        (format!("{folded}*"), DOCUMENTS),
        |row| row.opt_text(0),
    )
    .await?;
    Ok(texts.into_iter().flatten().collect())
}

/// Adds each word of one document's `texts` to `vocabulary`, once per
/// document however often it repeats: split where the tokenizer splits and
/// lowercased as it folds.
fn count_words<'a>(vocabulary: &mut HashMap<String, u64>, texts: impl Iterator<Item = &'a String>) {
    let mut seen: HashSet<String> = HashSet::new();
    for text in texts {
        for word in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
        {
            let word = word.to_lowercase();
            if seen.insert(word.clone()) {
                *vocabulary.entry(word).or_default() += 1;
            }
        }
    }
}

/// The first word of `text` that begins with `word`, lowercased.
fn begins_a_word(text: &str, word: &str) -> Option<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .find(|token| token.starts_with(word))
}

/// `(text, filter)` pairs as completions, each running its filter spelled
/// once (D13), uncounted.
fn completed(found: impl Iterator<Item = (String, Filter)>) -> Vec<Completion> {
    found
        .map(|(text, filter)| Completion {
            text,
            query: spell(&Clause {
                negated: false,
                filter,
            }),
            count: 0,
            capped: false,
        })
        .collect()
}

/// Every completion in `groups` counted as its query counts, in one
/// statement, each walk stopping at [`COMPLETION_COUNT_CAP`].
async fn count_all(
    connection: &Connection,
    account: AccountScope,
    groups: &mut [&mut Vec<Completion>],
) -> Result<()> {
    let queries: Vec<postio_search::ParsedQuery> = groups
        .iter()
        .flat_map(|group| group.iter())
        .map(|completion| postio_search::parse(&completion.query, undated()))
        .collect();
    let mut found = counts(connection, account, &queries, COMPLETION_COUNT_CAP)
        .await?
        .into_iter();
    for group in groups.iter_mut() {
        for completion in group.iter_mut() {
            let counted = found.next().unwrap_or_default();
            completion.count = counted.conversations;
            completion.capped = counted.capped;
        }
    }
    Ok(())
}

/// The ones that find something, most first, `most` of them: an offer that
/// finds nothing is not made.
fn keep_found(completions: &mut Vec<Completion>, most: usize) {
    completions.retain(|completion| completion.count > 0);
    completions.sort_by(|a, b| b.count.cmp(&a.count).then(a.text.cmp(&b.text)));
    completions.truncate(most);
}

/// `LIKE` patterns for a name with a word beginning with `prefix`: at its
/// start, or after a space.
fn word_patterns(prefix: &str) -> (String, String) {
    let escaped = escape_like(&prefix.to_lowercase());
    (format!("{escaped}%"), format!("% {escaped}%"))
}

/// The labels whose name has a word beginning with `prefix`, each name
/// once. One statement.
async fn label_names(
    connection: &Connection,
    account: AccountScope,
    prefix: &str,
) -> Result<Vec<String>> {
    let (start, inside) = word_patterns(prefix);
    let (scope, params) = match account.account() {
        Some(id) => (
            "AND account_id = ?3",
            vec![
                turso::Value::Text(start),
                turso::Value::Text(inside),
                turso::Value::Integer(id.get()),
            ],
        ),
        None => (
            "",
            vec![turso::Value::Text(start), turso::Value::Text(inside)],
        ),
    };
    let mut names: Vec<String> = sql::all(
        connection,
        &format!(
            "SELECT name FROM labels
              WHERE (lower(name) LIKE ?1 ESCAPE '\\' OR lower(name) LIKE ?2 ESCAPE '\\')
                {scope}
              ORDER BY name"
        ),
        params,
        |row| row.text(0),
    )
    .await?;
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names.truncate(LABELS * 4);
    Ok(names)
}

/// The folders whose name has a word beginning with `prefix`, each name
/// once, the ones mail is filed in. One statement.
async fn folder_names(
    connection: &Connection,
    account: AccountScope,
    prefix: &str,
) -> Result<Vec<String>> {
    let (start, inside) = word_patterns(prefix);
    let (scope, params) = match account.account() {
        Some(id) => (
            "AND account_id = ?3",
            vec![
                turso::Value::Text(start),
                turso::Value::Text(inside),
                turso::Value::Integer(id.get()),
            ],
        ),
        None => (
            "",
            vec![turso::Value::Text(start), turso::Value::Text(inside)],
        ),
    };
    let mut names: Vec<String> = sql::all(
        connection,
        &format!(
            "SELECT name FROM mailboxes
              WHERE selectable = 1
                AND role NOT IN ('drafts', 'junk', 'trash')
                AND (lower(name) LIKE ?1 ESCAPE '\\' OR lower(name) LIKE ?2 ESCAPE '\\')
                {scope}
              ORDER BY total_count DESC, name"
        ),
        params,
        |row| row.text(0),
    )
    .await?;
    let mut seen = HashSet::new();
    names.retain(|name| seen.insert(name.to_lowercase()));
    names.truncate(FOLDERS);
    Ok(names)
}

/// The people whose address or a word of whose name begins with `prefix`,
/// most written-to either way first, ties by the latest either way (D21).
/// Everyone, when nothing is typed yet. One statement.
async fn people(
    connection: &Connection,
    account: AccountScope,
    prefix: &str,
) -> Result<Vec<Person>> {
    let (start, inside) = word_patterns(prefix);
    let mut params = vec![
        turso::Value::Text(start),
        turso::Value::Text(inside),
        turso::Value::Integer(i64::try_from(PEOPLE).unwrap_or(4)),
    ];
    let scope = match account.account() {
        Some(id) => {
            params.push(turso::Value::Integer(id.get()));
            "AND (c.account_id = ?4 OR c.account_id IS NULL)"
        }
        None => "",
    };
    let sql = format!(
        "SELECT p.address, p.name, p.seen, coalesce(w.sent_count, 0), p.last_seen,
                w.last_sent_at
           FROM (SELECT c.address_normalized AS normalized, min(c.address) AS address,
                        max(coalesce(c.name, c.address_name)) AS name,
                        max(c.times_seen) AS seen, max(c.last_seen_at) AS last_seen
                   FROM contacts c
                  WHERE c.suppressed = 0 {scope}
                    AND (c.address_normalized LIKE ?1 ESCAPE '\\'
                         OR lower(coalesce(c.name, c.address_name, '')) LIKE ?1 ESCAPE '\\'
                         OR lower(coalesce(c.name, c.address_name, '')) LIKE ?2 ESCAPE '\\')
                  GROUP BY c.address_normalized) p
           LEFT JOIN addresses a ON a.address_normalized = p.normalized
           LEFT JOIN correspondents w ON w.address_id = a.id
          ORDER BY p.seen + coalesce(w.sent_count, 0) DESC,
                   max(coalesce(p.last_seen, 0), coalesce(w.last_sent_at, 0)) DESC,
                   p.address
          LIMIT ?3"
    );
    let found = sql::all(connection, &sql, params, |row| {
        let received: i64 = row.col::<Option<i64>>(2)?.unwrap_or(0);
        let sent: i64 = row.col::<Option<i64>>(3)?.unwrap_or(0);
        let last = [row.col::<Option<i64>>(4)?, row.col::<Option<i64>>(5)?]
            .into_iter()
            .flatten()
            .max()
            .and_then(DateTime::<Utc>::from_timestamp_millis);
        Ok(Person {
            address: row.text(0)?,
            name: row.opt_text(1)?.filter(|name| !name.is_empty()),
            received: u64::try_from(received).unwrap_or(0),
            sent: u64::try_from(sent).unwrap_or(0),
            last,
        })
    })
    .await?;
    Ok(found)
}
