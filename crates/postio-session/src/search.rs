//! Running a search, and cutting each hit's excerpt: how many hits one run brings back and which
//! text is highlighted are product decisions, so they live here once for every frontend.

use chrono::Utc;
use postio_index::executor::ConversationRequest;
use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, AddressId, EmailAddress, LabelId, MailboxId, MessageId};
use postio_search::facets::Scope;
use postio_search::passage::FirstLine;
use postio_search::results::{
    Addressee, ConversationKey, ConversationMatch, ConversationOrder, ConversationResults,
    FacetNames, Match, Source,
};
use postio_search::{ParsedQuery, ResultOrder, SearchResults};
use postio_storage::Checkout;

/// How many hits one run brings back: the page the result list is drawn from, not
/// `SearchResults::total_hits`.
pub const HIT_LIMIT: u32 = 200;

/// How many hits get an excerpt cut for them; short of `HIT_LIMIT` because each costs a blob read.
/// Past this the row uses the message's own preview.
const SNIPPET_HITS: usize = 50;

/// One search against the index, with an excerpt cut for each hit. `None` when the query could not
/// run at all; a query matching nothing is `Some` with no hits.
pub async fn execute(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    order: ResultOrder,
) -> Option<SearchResults> {
    execute_with_snippets(connection, account, query, scope, order, SNIPPET_HITS).await
}

/// `execute`, cutting an excerpt for only the first `snippets` hits: each is a body read, decode
/// and HTML-to-text pass, and fifty stood between a keystroke and the readout (#1613).
pub async fn execute_with_snippets(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    order: ResultOrder,
    snippets: usize,
) -> Option<SearchResults> {
    // Timed here on the app's clock: the whole wait including excerpts and the suggestion pass, and
    // a frozen storyboard clock must read the same each run.
    let started = postio_ui::clock::instant();
    let mut results = run(connection, account, query, scope, order).await?;

    // A word that found nothing is answered with the word that was meant, when the index offered
    // one and it finds something (ADR 0037). The offer is only for a single bare unquoted word, so
    // it is the query. Here, not in the executor, because saved searches and rules must match
    // exactly.
    let mut shown = std::borrow::Cow::Borrowed(query);
    if results.total_hits == 0
        && let Some(offer) = results.suggestion.clone()
    {
        let offered = postio_search::parse(&offer.term, postio_ui::clock::now().date_naive());
        if let Some(mut found) = run(connection, account, &offered, scope, order).await
            && found.total_hits > 0
        {
            let typed = query
                .text_terms()
                .next()
                .map(|term| term.value.clone())
                .unwrap_or_default();
            found.instead = Some(postio_search::Instead {
                typed,
                term: offer.term,
            });
            results = found;
            shown = std::borrow::Cow::Owned(offered);
        }
    }

    // Excerpts point at the word that matched, which after a rewrite is not the one typed.
    snippet_hits(connection, &shown, &mut results, snippets).await;
    results.elapsed = postio_ui::clock::instant().saturating_duration_since(started);
    Some(results)
}

async fn run(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    order: ResultOrder,
) -> Option<SearchResults> {
    search(
        connection,
        &SearchRequest {
            mailboxes: None,
            // The caller's own scope, passed through; it was once hardcoded to `Account` (#961).
            account,
            query,
            scope,
            limit: HIT_LIMIT,
            order,
        },
        postio_ui::clock::now().with_timezone(&Utc),
    )
    .await
    .map_err(|error| tracing::warn!(%error, "the search did not run"))
    .ok()
}

/// What the result set on screen is made of: the refine chips and the scope counts (#1157). Which
/// narrowings to offer is a product decision, so it lives here. A second pass because scope counts
/// ask what switching would find. `None` when the counts could not be taken.
pub async fn facets(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    order: ResultOrder,
) -> Option<postio_search::facets::Facets> {
    postio_index::executor::facets(
        connection,
        &SearchRequest {
            mailboxes: None,
            account,
            query,
            scope,
            limit: HIT_LIMIT,
            order,
        },
    )
    .await
    .map_err(|error| tracing::warn!(%error, "the facets could not be measured"))
    .ok()
}

/// Cuts each hit's excerpt out of its own body text. Here, not in the executor:
/// `message_bodies_fts` is `content = ''` (#407) and `postio-index` stays a rusqlite-only leaf
/// (#408).
///
/// It uses `postio_index::index::indexable_text`, the function the indexer uses, so the highlighted
/// string is the indexed one. A message with no local body gets no excerpt.
async fn snippet_hits(
    connection: &Checkout,
    query: &ParsedQuery,
    results: &mut SearchResults,
    snippets: usize,
) {
    let terms = postio_search::highlight::terms(query);
    if terms.is_empty() {
        return;
    }
    for hit in results.hits.iter_mut().take(snippets) {
        let body = crate::reading::load_body(connection, hit.message_id).await;
        if let Some(text) = postio_index::index::indexable_text(&body) {
            hit.snippet = postio_search::highlight::snippet(&text, &terms);
        }
    }
}

pub const FACET_NAMES: usize = 50;

/// One page of a conversation search, its facets' ids named. `None` when the query could not run.
/// GTK's search keeps its own path (spec 010 FR-046).
pub async fn conversations(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    order: ConversationOrder,
    offset: u32,
    limit: u32,
) -> Option<ConversationResults> {
    let started = postio_ui::clock::instant();
    let now = postio_ui::clock::now();
    let mut results = postio_index::executor::search_conversations(
        connection,
        &ConversationRequest {
            account,
            query,
            order,
            offset,
            limit,
            today: now.date_naive(),
        },
        now.with_timezone(&Utc),
    )
    .await
    .map_err(|error| tracing::warn!(%error, "the conversation search did not run"))
    .ok()?;

    results.facets.labels.truncate(FACET_NAMES);
    results.facets.folders.truncate(FACET_NAMES);
    // A missing name is a button that says less, never a failed search.
    results.names = names(connection, &results)
        .await
        .map_err(|error| tracing::warn!(%error, "the facets' names could not be read"))
        .unwrap_or_default();
    results.elapsed = postio_ui::clock::instant().saturating_duration_since(started);
    Some(results)
}

/// The Files tab's cards: attachments of the matching mail whose name or contents match, newest
/// first. `None` when the query could not run.
pub async fn files(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    offset: u32,
    limit: u32,
) -> Option<Vec<postio_search::results::FileHit>> {
    let now = postio_ui::clock::now();
    postio_index::executor::files(
        connection,
        &ConversationRequest {
            account,
            query,
            order: ConversationOrder::Newest,
            offset,
            limit,
            today: now.date_naive(),
        },
    )
    .await
    .map_err(|error| tracing::warn!(%error, "the files search did not run"))
    .ok()
}

/// The People tab's rows: who the matching mail is from and to, the person's own addresses left
/// out, most messages first. `None` when the query could not run.
pub async fn people(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    offset: u32,
    limit: u32,
) -> Option<Vec<postio_search::suggest::Person>> {
    let now = postio_ui::clock::now();
    postio_index::executor::people(
        connection,
        &ConversationRequest {
            account,
            query,
            order: ConversationOrder::Newest,
            offset,
            limit,
            today: now.date_naive(),
        },
    )
    .await
    .map_err(|error| tracing::warn!(%error, "the people search did not run"))
    .ok()
}

/// Every message of every conversation `query` matches except those whose best message is in
/// `except`: what ⇧X in Focus's results selects. Resolved with the results' own match and cap, so a
/// verb reaches exactly what was shown. `None` when the match could not be walked.
pub async fn matching(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    except: &[MessageId],
) -> Option<Vec<MessageId>> {
    let now = postio_ui::clock::now();
    let found = postio_index::executor::search_conversations(
        connection,
        &ConversationRequest {
            account,
            query,
            order: ConversationOrder::Newest,
            offset: 0,
            limit: u32::MAX,
            today: now.date_naive(),
        },
        now.with_timezone(&Utc),
    )
    .await
    .map_err(|error| tracing::warn!(%error, "the match to act on could not be walked"))
    .ok()?;
    let threads = postio_storage::repository::ThreadRepository::new(connection);
    let mut messages = Vec::new();
    for hit in found.hits.iter().filter(|hit| !except.contains(&hit.best)) {
        match hit.key {
            ConversationKey::Thread(thread) => messages.extend(
                threads
                    .messages(thread, postio_storage::repository::ThreadOrder::Oldest)
                    .await
                    .map_err(|error| tracing::warn!(%error, "a matched thread could not be read"))
                    .ok()?
                    .into_iter()
                    .map(|row| row.id),
            ),
            ConversationKey::Lone(message) => messages.push(message),
        }
    }
    Some(messages)
}

/// The names behind `results`' ids: one read per kind, however many there are. A person's name is
/// one their mail carried.
async fn names(
    connection: &Checkout,
    results: &ConversationResults,
) -> postio_storage::Result<FacetNames> {
    use postio_storage::sql::{self, RowExt as _};

    fn json(ids: impl Iterator<Item = i64>) -> String {
        let mut ids: Vec<i64> = ids.collect();
        ids.sort_unstable();
        ids.dedup();
        format!(
            "[{}]",
            ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
        )
    }

    let facets = &results.facets;
    let people = json(
        facets
            .senders
            .iter()
            .chain(&facets.recipients)
            .map(|count| count.id.get()),
    );
    let labels = json(
        facets.labels.iter().map(|count| count.id.get()).chain(
            results
                .hits
                .iter()
                .flat_map(|hit| hit.labels.iter().map(|id| id.get())),
        ),
    );
    let folders = json(facets.folders.iter().map(|count| count.id.get()));

    let named = sql::all(
        connection,
        &format!(
            "SELECT a.id, a.address,
                    (SELECT r.name FROM recipients r
                      WHERE r.address_id = a.id AND r.name IS NOT NULL AND r.name <> ''
                      LIMIT 1),
                    a.address_normalized IN ({})
               FROM json_each(?1) j JOIN addresses a ON a.id = j.value",
            postio_index::executor::OWN_ADDRESSES
        ),
        [people.as_str()],
        |row| {
            Ok((
                AddressId::new(row.int(0)?),
                EmailAddress {
                    name: row.opt_text(2)?,
                    address: row.text(1)?,
                },
                row.int(3)? != 0,
            ))
        },
    )
    .await?;
    let own = named
        .iter()
        .filter(|(_, _, own)| *own)
        .map(|(id, _, _)| *id)
        .collect();
    let people = named
        .into_iter()
        .map(|(id, address, _)| (id, address))
        .collect();
    let labelled = sql::all(
        connection,
        "SELECT l.id, l.name, l.color FROM json_each(?1) j JOIN labels l ON l.id = j.value",
        [labels.as_str()],
        |row| Ok((LabelId::new(row.int(0)?), row.text(1)?, row.opt_text(2)?)),
    )
    .await?;
    let label_colors = labelled
        .iter()
        .filter_map(|(id, _, color)| Some((*id, color.clone()?)))
        .collect();
    let labels = labelled
        .into_iter()
        .map(|(id, name, _)| (id, name))
        .collect();
    let folders = sql::all(
        connection,
        "SELECT m.id, m.name FROM json_each(?1) j JOIN mailboxes m ON m.id = j.value",
        [folders.as_str()],
        |row| Ok((MailboxId::new(row.int(0)?), row.text(1)?)),
    )
    .await?;
    let best: Vec<MessageId> = results.hits.iter().map(|hit| hit.best).collect();
    let held = postio_storage::repository::DigestRepository::new(connection)
        .held(&best)
        .await?;
    Ok(FacetNames {
        people,
        own,
        labels,
        label_colors,
        folders,
        held,
    })
}

/// The passages of each hit's matches, cut from its own body, each body match told apart into the
/// person's own words and the quoted history (spec 010 D6, D7). `first_line` says whether the row
/// shows the message's first line as its preview.
///
/// One body read per hit that matched in its body; one read for the whole page's attachments. A
/// message with no local body keeps its sources without passages.
/// Quoted is decided here because the index does not know where a quote starts; the reader's quote
/// detector over `indexable_text` says it exactly, only for the page on screen.
pub async fn passages(
    connection: &Checkout,
    query: &ParsedQuery,
    hits: &[(MessageId, Vec<Source>)],
    first_line: FirstLine,
) -> Vec<(MessageId, Vec<Match>)> {
    let terms: Vec<String> = query
        .searchable_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.clone())
        .filter(|value| !value.is_empty())
        .collect();
    let wanted: Vec<_> = hits
        .iter()
        .flat_map(|(_, sources)| sources)
        .filter_map(|source| match source {
            Source::FileContent {
                attachment,
                location,
                ..
            } => Some((*attachment, location.clone())),
            _ => None,
        })
        .collect();
    let units = if wanted.is_empty() || terms.is_empty() {
        Vec::new()
    } else {
        postio_index::index::attachment_units(connection, &wanted)
            .await
            .map_err(|error| tracing::warn!(%error, "attachment passages could not be read"))
            .unwrap_or_default()
    };
    let mut answered = Vec::with_capacity(hits.len());
    for (message, sources) in hits {
        let text = if sources.contains(&Source::Body) && !terms.is_empty() {
            let body = crate::reading::load_body(connection, *message).await;
            postio_index::index::indexable_text(&body)
        } else {
            None
        };
        let mut matches = Vec::with_capacity(sources.len() + 1);
        for source in sources {
            match (source, &text) {
                (Source::Body, Some(text)) => {
                    matches.extend(body_matches(text, &terms, first_line));
                }
                (
                    Source::FileContent {
                        attachment,
                        location,
                        ..
                    },
                    _,
                ) => matches.push(Match {
                    source: source.clone(),
                    passage: units
                        .iter()
                        .find(|(unit, at, _)| unit == attachment && at == location)
                        .and_then(|(_, _, text)| {
                            postio_search::passage::cut(text, &terms, FirstLine::Any)
                        }),
                    when: None,
                }),
                (source, _) => matches.push(Match {
                    source: source.clone(),
                    passage: None,
                    when: None,
                }),
            }
        }
        answered.push((*message, matches));
    }
    answered
}

struct Told {
    own: String,
    quoted: String,
    opens_own: bool,
    opens_quoted: bool,
}

impl Told {
    fn of(text: &str) -> Self {
        use postio_body::quote::{Stretch, text_stretches};

        let stretches = text_stretches(text);
        let mut own = String::new();
        let mut quoted = String::new();
        let mut opens_own = false;
        for (at, stretch) in stretches.iter().enumerate() {
            match *stretch {
                Stretch::Own(words) => {
                    // The attribution line ("On Fri, Ada wrote:") is neither the person's words nor
                    // the history.
                    let words = match stretches.get(at + 1) {
                        Some(Stretch::Quoted(_)) => postio_body::quote::without_attribution(words),
                        _ => words,
                    };
                    opens_own |= at == 0 && !words.trim().is_empty();
                    own.push_str(words);
                }
                Stretch::Quoted(lines) => quoted.push_str(&unquoted(lines)),
            }
        }
        Told {
            own,
            quoted,
            opens_own,
            opens_quoted: matches!(stretches.first(), Some(Stretch::Quoted(_))),
        }
    }

    fn own_match(&self, terms: &[String], first_line: FirstLine) -> Option<Match> {
        found(&self.own, terms).then(|| Match {
            source: Source::Body,
            passage: postio_search::passage::cut(
                &self.own,
                terms,
                if self.opens_own {
                    first_line
                } else {
                    FirstLine::Any
                },
            ),
            when: None,
        })
    }

    fn quoted_match(&self, terms: &[String], first_line: FirstLine) -> Option<Match> {
        found(&self.quoted, terms).then(|| Match {
            source: Source::Quoted,
            passage: postio_search::passage::cut(
                &self.quoted,
                terms,
                if self.opens_quoted {
                    first_line
                } else {
                    FirstLine::Any
                },
            ),
            when: None,
        })
    }
}

fn found(words: &str, terms: &[String]) -> bool {
    !postio_search::highlight::find(words, terms).is_empty()
}

fn body_matches(text: &str, terms: &[String], first_line: FirstLine) -> Vec<Match> {
    // The message's first line is the row's preview (D7), never the passage unless it was an
    // attribution.
    let told = Told::of(text);
    let mut matches: Vec<Match> = told
        .own_match(terms, first_line)
        .into_iter()
        .chain(told.quoted_match(terms, first_line))
        .collect();
    if matches.is_empty() {
        matches.push(Match {
            source: Source::Body,
            passage: None,
            when: None,
        });
    }
    matches
}

const MATCHED_MESSAGES: usize = 50;

/// Every match in one conversation, for Quick Look (FR-028): each message's own words oldest first
/// with who and when, its quoted history when the conversation does not hold what it quotes, the
/// matching file names, and the subject last.
///
/// A quote found in another message's own words is left out when that message has its own match; a
/// message whose only match is a quote keeps it, or "K of M" would lie (#2184). Empty when the
/// conversation could not be read.
pub async fn conversation_matches(
    connection: &Checkout,
    query: &ParsedQuery,
    key: ConversationKey,
) -> Vec<ConversationMatch> {
    match read_matches(connection, query, key).await {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(%error, "a conversation's matches could not be read");
            Vec::new()
        }
    }
}

async fn read_matches(
    connection: &Checkout,
    query: &ParsedQuery,
    key: ConversationKey,
) -> postio_storage::Result<Vec<ConversationMatch>> {
    use postio_search::query::Filter;
    use postio_storage::repository::{MessageRepository, ThreadOrder, ThreadRepository};

    let mut members = match key {
        ConversationKey::Thread(thread) => {
            ThreadRepository::new(connection)
                .messages(thread, ThreadOrder::Oldest)
                .await?
        }
        ConversationKey::Lone(message) => {
            MessageRepository::new(connection)
                .rows_for(&[message])
                .await?
        }
    };
    if members.len() > MATCHED_MESSAGES {
        members.drain(..members.len() - MATCHED_MESSAGES);
    }

    let words: Vec<String> = query
        .searchable_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.clone())
        .filter(|value| !value.is_empty())
        .collect();
    let with = |pick: fn(&Filter) -> Option<&String>| {
        let mut terms = words.clone();
        terms.extend(
            query
                .filters()
                .filter(|clause| !clause.negated)
                .flat_map(|clause| clause.filter.alternatives())
                .filter_map(|filter| pick(filter).cloned()),
        );
        terms
    };
    let subject_terms = with(|filter| match filter {
        Filter::Subject(value) => Some(value),
        _ => None,
    });
    let file_terms = with(|filter| match filter {
        Filter::Filename(value) => Some(value),
        _ => None,
    });

    let mut told = Vec::with_capacity(members.len());
    for member in &members {
        let text = if words.is_empty() {
            None
        } else {
            let body = crate::reading::load_body(connection, member.id).await;
            postio_index::index::indexable_text(&body)
        };
        told.push(text.as_deref().map(Told::of));
    }
    let files = if file_terms.is_empty() {
        Vec::new()
    } else {
        file_names(connection, members.iter().map(|member| member.id)).await?
    };
    let read = if words.is_empty() {
        Vec::new()
    } else {
        let ids: Vec<MessageId> = members.iter().map(|member| member.id).collect();
        postio_index::executor::file_matches(connection, query, &ids)
            .await
            .map_err(
                |error| tracing::warn!(%error, "a conversation's file matches could not be read"),
            )
            .unwrap_or_default()
    };

    let addressed = sent_to(connection, members.iter().map(|member| member.id)).await?;
    let mut found = Vec::new();
    for (at, member) in members.iter().enumerate() {
        let when = Some(member.received_at);
        let to: Vec<Addressee> = addressed
            .iter()
            .filter(|(of, _)| *of == member.id)
            .map(|(_, each)| each.clone())
            .collect();
        let mut in_files = Vec::new();
        for (_, attachment, name) in files.iter().filter(|(of, attachment, _)| {
            *of == member.id && !read.iter().any(|file| file.attachment == *attachment)
        }) {
            let marks = postio_search::highlight::find(name, &file_terms);
            if marks.is_empty() {
                continue;
            }
            in_files.push(ConversationMatch {
                message: Some(member.id),
                to: to.clone(),
                from: member.from.clone(),
                found: Match {
                    source: Source::FileName {
                        attachment: *attachment,
                        name: name.clone(),
                    },
                    passage: Some(whole(name, marks)),
                    when,
                },
            });
        }
        for file in read.iter().filter(|file| file.message == member.id) {
            in_files.push(ConversationMatch {
                message: Some(member.id),
                to: to.clone(),
                from: member.from.clone(),
                found: Match {
                    source: Source::FileContent {
                        attachment: file.attachment,
                        name: file.name.clone(),
                        location: file.location.clone(),
                    },
                    passage: postio_search::passage::cut(&file.text, &words, FirstLine::Any),
                    when,
                },
            });
        }
        if let Some(body) = &told[at] {
            let own = body.own_match(&words, FirstLine::Avoided);
            let has_own = own.is_some() || !in_files.is_empty();
            if let Some(mut own) = own {
                own.when = when;
                found.push(ConversationMatch {
                    message: Some(member.id),
                    to: to.clone(),
                    from: member.from.clone(),
                    found: own,
                });
            }
            let quoted = body
                .quoted_match(&words, FirstLine::Avoided)
                .filter(|quoted| {
                    !has_own
                        || !told.iter().enumerate().any(|(other, body)| {
                            other != at
                                && body.as_ref().is_some_and(|body| said_in(quoted, &body.own))
                        })
                });
            if let Some(quoted) = quoted {
                found.push(ConversationMatch {
                    message: Some(member.id),
                    to: to.clone(),
                    from: None,
                    found: quoted,
                });
            }
        }
        found.append(&mut in_files);
    }
    let subject = members.iter().find_map(|member| {
        let subject = member.subject.as_deref()?;
        let marks = postio_search::highlight::find(subject, &subject_terms);
        (!marks.is_empty()).then(|| whole(subject, marks))
    });
    if let Some(subject) = subject {
        found.push(ConversationMatch {
            message: None,
            to: Vec::new(),
            from: None,
            found: Match {
                source: Source::Subject,
                passage: Some(subject),
                when: None,
            },
        });
    }
    Ok(found)
}

/// Whether a quoted match's words are among `own`, a message's own words, compared with case and
/// spacing folded because a quote rewraps lines.
fn said_in(quoted: &Match, own: &str) -> bool {
    let fold = |text: &str| {
        text.split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
            .join(" ")
    };
    quoted
        .passage
        .as_ref()
        .is_some_and(|passage| fold(own).contains(&fold(&passage.text)))
}

fn whole(text: &str, marks: Vec<std::ops::Range<usize>>) -> postio_search::results::Passage {
    postio_search::results::Passage {
        text: text.to_owned(),
        ranges: marks,
        elided_start: false,
        elided_end: false,
    }
}

/// Who each of `messages` was sent to: its To line in order, each marked when it is the person's
/// own address. One read, on `idx_recipients_message`.
async fn sent_to(
    connection: &Checkout,
    messages: impl Iterator<Item = MessageId>,
) -> postio_storage::Result<Vec<(MessageId, Addressee)>> {
    use postio_storage::sql::{self, RowExt as _};
    let ids = format!(
        "[{}]",
        messages
            .map(|id| id.get().to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    sql::all(
        connection,
        &format!(
            "SELECT r.message_id, a.address, r.name,
                    a.address_normalized IN ({})
               FROM json_each(?1) j
               JOIN recipients r ON r.message_id = j.value AND r.kind = 'to'
               JOIN addresses a ON a.id = r.address_id
              ORDER BY r.message_id, r.position",
            postio_index::executor::OWN_ADDRESSES
        ),
        [ids.as_str()],
        |row| {
            Ok((
                MessageId::new(row.int(0)?),
                Addressee {
                    address: EmailAddress {
                        name: row.opt_text(2)?,
                        address: row.text(1)?,
                    },
                    own: row.int(3)? != 0,
                },
            ))
        },
    )
    .await
}

async fn file_names(
    connection: &Checkout,
    messages: impl Iterator<Item = MessageId>,
) -> postio_storage::Result<Vec<(MessageId, postio_model::AttachmentId, String)>> {
    use postio_storage::sql::{self, RowExt as _};
    let ids = format!(
        "[{}]",
        messages
            .map(|id| id.get().to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    sql::all(
        connection,
        "SELECT f.message_id, f.id, f.filename
           FROM json_each(?1) j JOIN attachments f ON f.message_id = j.value
          WHERE f.filename IS NOT NULL
          ORDER BY f.message_id, f.id",
        [ids.as_str()],
        |row| {
            Ok((
                MessageId::new(row.int(0)?),
                postio_model::AttachmentId::new(row.int(1)?),
                row.text(2)?,
            ))
        },
    )
    .await
}

fn unquoted(lines: &str) -> String {
    lines
        .split_inclusive('\n')
        .map(|line| line.trim_start_matches(['>', ' ', '\t']))
        .collect()
}

/// What `prefix` could become (US7): the words, labels, lists and files it begins, or `field`'s
/// values, each counted. `None` when the index could not be read.
pub async fn suggest(
    connection: &Checkout,
    account: AccountScope,
    prefix: &str,
    field: Option<postio_search::query::Field>,
) -> Option<postio_search::suggest::Suggestions> {
    postio_index::executor::completions(connection, account, prefix, field)
        .await
        .map_err(|error| tracing::warn!(%error, "the completions could not be read"))
        .ok()
}

/// The looser searches a query that found nothing offers, each with the conversations it would
/// find: those finding none left out, most first (US6). `None` when the counts could not be taken.
pub async fn relaxations(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    today: chrono::NaiveDate,
) -> Option<
    Vec<(
        postio_search::relax::Relaxation,
        postio_search::results::Counted,
    )>,
> {
    let offered = postio_search::relax::relax(query);
    let counts = postio_index::executor::relaxation_counts(connection, account, &offered, today)
        .await
        .map_err(|error| tracing::warn!(%error, "the relaxations could not be counted"))
        .ok()?;
    let mut counted: Vec<_> = offered
        .into_iter()
        .zip(counts)
        .filter(|(_, count)| count.conversations > 0)
        .collect();
    counted.sort_by_key(|(_, count)| std::cmp::Reverse(count.conversations));
    Some(counted)
}

pub async fn page(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    mailboxes: &[MailboxId],
    order: ResultOrder,
    offset: u32,
    limit: u32,
) -> Option<postio_search::results::SearchPage> {
    let cap = postio_search::results::TOTAL_HITS_CAP as u32;
    let offset = if order == ResultOrder::Newest {
        offset.min(cap)
    } else {
        0
    };
    let results = limited(
        connection,
        account,
        query,
        mailboxes,
        order,
        (offset + limit).min(cap),
    )
    .await?;
    let ids: Vec<MessageId> = results
        .hits
        .iter()
        .skip(offset as usize)
        .map(|hit| hit.message_id)
        .collect();
    let reached = offset + ids.len() as u32;
    Some(postio_search::results::SearchPage {
        next: (order == ResultOrder::Newest
            && (reached as u64) < results.total_hits
            && reached < cap)
            .then_some(reached),
        ids,
        total: results.total_hits,
        total_capped: results.total_hits_capped,
    })
}

pub async fn stats(
    connection: &Checkout,
    account: AccountScope,
    query: &str,
    mailboxes: &[MailboxId],
    group_by: postio_search::results::GroupBy,
    groups: usize,
    today: chrono::NaiveDate,
) -> Option<postio_search::results::MailStats> {
    use postio_search::results::{GroupBy, MailStats, STATS_CAP, StatsGroup};
    use std::collections::{HashMap, HashSet};

    let base = limited(
        connection,
        account,
        &postio_search::parse(query, today),
        mailboxes,
        ResultOrder::Newest,
        STATS_CAP,
    )
    .await?;
    let mut sets: Vec<HashSet<MessageId>> = Vec::with_capacity(4);
    for state in ["is:unread", "is:flagged", "is:automated", "is:replied"] {
        let narrowed = postio_search::parse(&format!("{query} {state}"), today);
        let found = limited(
            connection,
            account,
            &narrowed,
            mailboxes,
            ResultOrder::Newest,
            STATS_CAP,
        )
        .await?;
        sets.push(found.hits.iter().map(|hit| hit.message_id).collect());
    }
    let own: HashSet<String> = postio_storage::repository::IdentityRepository::new(connection)
        .own_addresses()
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the person's own addresses"))
        .ok()?
        .into_iter()
        .map(|address| address.address.to_ascii_lowercase())
        .collect();

    let mut by_key: HashMap<String, StatsGroup> = HashMap::new();
    for hit in &base.hits {
        let address = hit
            .from
            .as_ref()
            .map(|from| from.address.to_ascii_lowercase())
            .unwrap_or_default();
        let key = match group_by {
            GroupBy::Sender if own.contains(&address) => continue,
            GroupBy::Domain if own.contains(&address) => continue,
            GroupBy::Sender => address,
            GroupBy::Domain => address
                .rsplit_once('@')
                .map(|(_, domain)| domain.to_owned())
                .unwrap_or(address),
            GroupBy::Month => hit.received_at.format("%Y-%m").to_string(),
        };
        let group = by_key.entry(key.clone()).or_insert_with(|| StatsGroup {
            key,
            count: 0,
            first: hit.received_at,
            last: hit.received_at,
            unread: 0,
            flagged: 0,
            automated: 0,
            replied: 0,
        });
        group.count += 1;
        group.first = group.first.min(hit.received_at);
        group.last = group.last.max(hit.received_at);
        let has = |set: usize| u32::from(sets[set].contains(&hit.message_id));
        group.unread += has(0);
        group.flagged += has(1);
        group.automated += has(2);
        group.replied += has(3);
    }
    let mut groups_found: Vec<StatsGroup> = by_key.into_values().collect();
    groups_found.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    groups_found.truncate(groups);
    let counted = base.hits.len() as u32;
    Some(MailStats {
        groups: groups_found,
        total: base.total_hits,
        counted,
        capped: base.total_hits_capped || u64::from(counted) < base.total_hits,
    })
}

async fn limited(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    mailboxes: &[MailboxId],
    order: ResultOrder,
    limit: u32,
) -> Option<SearchResults> {
    search(
        connection,
        &SearchRequest {
            account,
            query,
            scope: Scope::AllMail,
            mailboxes: Some(mailboxes),
            limit,
            order,
        },
        postio_ui::clock::now().with_timezone(&Utc),
    )
    .await
    .map_err(|error| tracing::warn!(%error, "the search did not run"))
    .ok()
}
