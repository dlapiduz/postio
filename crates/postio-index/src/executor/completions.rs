//! What a prefix could become, for the dropdown's short-prefix and operator states. The index has
//! no term dictionary, so words are recovered as [`super::words_near`] does:
//! metadata first, bodies only when it gives fewer than [`ENOUGH_WORDS`]. Every count is what the
//! suggestion's query would find, in one statement, up to [`COMPLETION_COUNT_CAP`] (a floor past
//! it).
//! People for `from:` and `to:` rank by how often they wrote to you plus how often you wrote to
//! them.

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

const DOCUMENTS: i64 = 50;

const ENOUGH_WORDS: usize = 3;

const WORDS: usize = 1;

const LABELS: usize = 3;
const FOLDERS: usize = 4;
const LISTS: usize = 1;

const FILES: usize = 20;

const PEOPLE: usize = 4;

/// Past it the count is a floor and the row says "N+". A completion as common as `as` walked the
/// whole mailbox to the search's own cap and cost 28 ms of a 20 ms keystroke budget.
pub const COMPLETION_COUNT_CAP: u64 = 1_000;

const UNIT: char = '\u{1f}';

/// The counted queries name no dates, so any day reads them the same.
fn undated() -> NaiveDate {
    NaiveDate::default()
}

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

struct Document {
    texts: Vec<Option<String>>,
    list: Option<String>,
    /// Whole: the index's own column joins names with spaces, which a name may hold.
    files: Vec<String>,
}

/// `fts_score` is projected bare with the match's own parameter, or it answers 0.0 (see
/// `HITS_JOIN`); file names are read for those documents only, outside the ordered match.
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

fn begins_a_word(text: &str, word: &str) -> Option<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .find(|token| token.starts_with(word))
}

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

/// An offer that finds nothing is not made.
fn keep_found(completions: &mut Vec<Completion>, most: usize) {
    completions.retain(|completion| completion.count > 0);
    completions.sort_by(|a, b| b.count.cmp(&a.count).then(a.text.cmp(&b.text)));
    completions.truncate(most);
}

fn word_patterns(prefix: &str) -> (String, String) {
    let escaped = escape_like(&prefix.to_lowercase());
    (format!("{escaped}%"), format!("% {escaped}%"))
}

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

/// Your own addresses are left out, as the People tab leaves them out: they are on every message
/// you sent and would always rank first.
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
            "AND s.account_id = ?4"
        }
        None => "",
    };
    // Evidence is per address and account, and a person hides an address only by not being live: an
    // address nobody owns yet is offered.
    let sql = format!(
        "SELECT p.address, p.name, p.seen, coalesce(w.sent_count, 0), p.last_seen,
                w.last_sent_at
           FROM (SELECT a.id AS address_id, a.address_normalized AS normalized,
                        a.address AS address,
                        max(coalesce(nullif(c.name, ''), s.last_name)) AS name,
                        sum(s.times_seen) AS seen, max(s.last_seen_at) AS last_seen
                   FROM contact_sightings s
                   JOIN addresses a ON a.id = s.address_id
                   LEFT JOIN contacts c ON c.id = a.contact_id
                  WHERE (c.id IS NULL OR c.state = 'live') {scope}
                    AND a.address_normalized NOT IN ({own})
                    AND (a.address_normalized LIKE ?1 ESCAPE '\\'
                         OR lower(coalesce(nullif(c.name, ''), s.last_name, '')) LIKE ?1 ESCAPE '\\'
                         OR lower(coalesce(nullif(c.name, ''), s.last_name, '')) LIKE ?2 ESCAPE '\\')
                  GROUP BY a.id) p
           LEFT JOIN correspondents w ON w.address_id = p.address_id
          ORDER BY p.seen + coalesce(w.sent_count, 0) DESC,
                   max(coalesce(p.last_seen, 0), coalesce(w.last_sent_at, 0)) DESC,
                   p.address
          LIMIT ?3",
        own = super::people::OWN_ADDRESSES
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
