//! Running a search, and cutting each hit's excerpt.
//!
//! `postio_index::search` is the executor and was always reachable from
//! anywhere; this is the thin layer above it that decides how many hits one
//! run brings back and reconstructs the excerpt each hit shows. That layer
//! lived in the classic app until #660, where only the GTK build could reach it.
//!
//! It is here rather than there because it is a *product* decision — how many
//! results, and which text gets highlighted — and two copies of a product
//! decision are two products. `docs/PRODUCT.md` §1 puts finding things among
//! the three jobs Postio must beat the alternatives at; a macOS build with its
//! own hit limit and its own excerpt rule would be answering the same query
//! differently the first time either copy was edited.

use chrono::Utc;
use postio_index::executor::ConversationRequest;
use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, AddressId, EmailAddress, LabelId, MailboxId, MessageId};
use postio_search::facets::Scope;
use postio_search::passage::FirstLine;
use postio_search::results::{
    ConversationKey, ConversationMatch, ConversationOrder, ConversationResults, FacetNames, Match,
    Source,
};
use postio_search::{ParsedQuery, ResultOrder, SearchResults};
use postio_storage::Checkout;

/// How many hits one run brings back.
///
/// Not how many *matched* — that is [`SearchResults::total_hits`], which the
/// readout draws and which counts far past this. This is the page the result
/// list is drawn from, and nobody scrolls two hundred results looking for the
/// one they meant.
pub const HIT_LIMIT: u32 = 200;

/// How many hits get an excerpt cut for them.
///
/// Several screens' worth, and short of [`HIT_LIMIT`] on purpose. Each one
/// costs a blob read, and a person who scrolls past fifty results without
/// refining the query is doing something a snippet was not going to help
/// with. Past this the row falls back to the message's own preview, which it
/// already has.
const SNIPPET_HITS: usize = 50;

/// One search against the index, with an excerpt cut for each hit.
///
/// `None` when the query could not run at all — a corrupt index, a locked
/// database. A query that simply matches nothing is `Some` with no hits,
/// because those are different answers and the surface says different things
/// about them.
pub async fn execute(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    order: ResultOrder,
) -> Option<SearchResults> {
    execute_with_snippets(connection, account, query, scope, order, SNIPPET_HITS).await
}

/// [`execute`], cutting an excerpt for only the first `snippets` hits.
///
/// For a surface that draws one excerpt at a time: the GTK finder shows the
/// focused hit's in its preview, and only while that hit's body is still on
/// its way -- the body replaces it -- so it asks for the best match's alone.
/// Each excerpt is a body read, a decode and an HTML-to-text pass, and fifty
/// of them stood between a keystroke and the readout's answer (#1613).
pub async fn execute_with_snippets(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    order: ResultOrder,
    snippets: usize,
) -> Option<SearchResults> {
    // Timed here, on the app's clock, rather than trusting the executor's
    // own figure: this is the whole wait the readout reports, excerpts and
    // the suggestion's second pass included, and a frozen clock (a
    // storyboard's) has to read the same in every run.
    let started = postio_ui::clock::instant();
    let mut results = run(connection, account, query, scope, order).await?;

    // A word that found nothing is answered with the word that was meant,
    // when the index offered one and it finds something here (ADR 0037,
    // amended). The box keeps what was typed; `instead` is what lets the
    // surface say the list is for a different word. The executor only ever
    // offers for a single bare, unquoted word, so the offer *is* the query.
    //
    // Here and not in the executor, because the executor also answers saved
    // searches and rules, which must match exactly what they say.
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

    // Excerpts point at the word that matched, which after a rewrite is not
    // the one typed.
    snippet_hits(connection, &shown, &mut results, snippets).await;
    results.elapsed = postio_ui::clock::instant().saturating_duration_since(started);
    Some(results)
}

/// One run of the executor for the box: the caller's scope, the hit limit.
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
            // The caller's own scope, passed through. It was hardcoded to
            // `Account` while the composition root opened exactly one account
            // and there was nothing to observe the difference with; #185 gave
            // the sidebar a section per account and a Unified row, and the
            // hardcode outlived its reason by long enough that the executor's
            // unified path had a benchmark and no caller (#961). Moving this
            // function must not put it back.
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

/// What the result set on screen is made of — the refine chips and the scope
/// counts (#1157).
///
/// Beside [`execute`] and for the same reason its module doc gives: which
/// narrowings are worth offering is a *product* decision, and two frontends
/// each choosing four chips out of the same measurements would be offering
/// two different query languages the first time either copy was edited.
///
/// A second pass over the index rather than a field on [`SearchResults`],
/// because it is a different question — the scope counts ask what
/// *switching* would find, which cannot be measured inside the scope you are
/// already in — and because a run that only draws a list should not pay for
/// it.
///
/// `None` when the counts could not be taken. The chips are an offer, and an
/// offer that cannot be made is simply not made.
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

/// Cuts each hit's excerpt out of its own body text.
///
/// # Why this is here and not in the executor
///
/// The boundary #408 settled. `snippet()` was an FTS5 function over indexed
/// content and the body index has none — `message_bodies_fts` is
/// `content = ''`, which is the point of it (#407). Reconstructing an excerpt
/// needs the body, the body is in the blob store, and `postio-index` is a
/// rusqlite-only leaf that `check-crate-boundaries.py` keeps that way.
///
/// # Why it agrees with what matched
///
/// `postio_index::index::indexable_text` is the function the *indexer* uses to
/// decide what a message's searchable text is — `text/plain` when there is
/// one, the HTML rendered to text otherwise. Calling the same function means
/// the string highlighted is the string that was indexed, rather than a second
/// guess at it, and `postio_search::highlight`'s token rule is FTS5's own. A
/// message with no local body gets no excerpt rather than a wrong one.
async fn snippet_hits(
    connection: &Checkout,
    query: &ParsedQuery,
    results: &mut SearchResults,
    snippets: usize,
) {
    let terms = postio_search::highlight::terms(query);
    if terms.is_empty() {
        // A structured-only query — `is:unread`, `in:archive` — has nothing to
        // point at, and every hit's snippet stays empty exactly as it did when
        // SQLite was cutting them.
        return;
    }
    for hit in results.hits.iter_mut().take(snippets) {
        let body = crate::reading::load_body(connection, hit.message_id).await;
        if let Some(text) = postio_index::index::indexable_text(&body) {
            hit.snippet = postio_search::highlight::snippet(&text, &terms);
        }
    }
}

// ---------------------------------------------------------------------------
// Conversation search (spec 010): Focus's results view
// ---------------------------------------------------------------------------

/// How many values of one facet get a name: the popovers' lists.
///
/// The executor keeps the top fifty people already; labels and folders are
/// held to the same, so one read per kind names everything a popover shows.
pub const FACET_NAMES: usize = 50;

/// One page of a conversation search, its facets' ids named.
///
/// Beside [`execute`], not instead of it: GTK's search keeps its own path
/// (spec 010 FR-046). `None` when the query could not run, as there.
pub async fn conversations(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    order: ConversationOrder,
    offset: u32,
    limit: u32,
) -> Option<ConversationResults> {
    // On the app's clock, as `execute` is, and for the same reason: a
    // storyboard freezes it, and the months end with *its* today.
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
    // A missing name is a button that says less, never a search that
    // failed: the counts stand without them.
    results.names = names(connection, &results)
        .await
        .map_err(|error| tracing::warn!(%error, "the facets' names could not be read"))
        .unwrap_or_default();
    results.elapsed = postio_ui::clock::instant().saturating_duration_since(started);
    Some(results)
}

/// Every message of every conversation `query` matches, but those whose
/// best message is in `except`: what ⇧X in Focus's results selects (spec
/// 010 US5), resolved with the results' own match -- capped as their count
/// is -- so a verb on it reaches exactly what the person was shown. `None`
/// when the match could not be walked.
///
/// A conversation is every message of its thread, as a verb on one row
/// is; one on no thread is its message.
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

/// The names behind `results`' ids: one read per kind -- people, labels,
/// folders -- however many there are.
///
/// A person's name is one their mail carried; the address row keeps only
/// the address.
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

    let people = sql::all(
        connection,
        "SELECT a.id, a.address,
                (SELECT r.name FROM recipients r
                  WHERE r.address_id = a.id AND r.name IS NOT NULL AND r.name <> ''
                  LIMIT 1)
           FROM json_each(?1) j JOIN addresses a ON a.id = j.value",
        [people.as_str()],
        |row| {
            Ok((
                AddressId::new(row.int(0)?),
                EmailAddress {
                    name: row.opt_text(2)?,
                    address: row.text(1)?,
                },
            ))
        },
    )
    .await?;
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
    Ok(FacetNames {
        people,
        labels,
        label_colors,
        folders,
    })
}

/// The passages of each hit's matches, cut from its own body, each body
/// match told apart into the person's own words and the history they quoted
/// (spec 010 D6, D7). `first_line` is what the asking row does with the
/// message's first line: the dropdown shows it as the preview
/// ([`FirstLine::Shown`]), the results view shows none
/// ([`FirstLine::Avoided`]).
///
/// One body read per hit that matched in its body, and none for the rest:
/// a subject is drawn by the row itself and a file name is its own words.
/// The answer keeps the order of `hits` and, within each, of its sources,
/// a body match becoming [`Source::Body`], [`Source::Quoted`] or both. A
/// message with no local body keeps its sources without passages: the
/// search found it by words this machine no longer has.
///
/// # Why quoted is decided here
///
/// The index does not know where a quote starts, and teaching it would mean
/// a second indexed column and a reindex of every body. The quote detector
/// the reader folds by, over [`postio_index::index::indexable_text`] -- the
/// very text the index holds -- says it exactly, and only for the page on
/// screen.
pub async fn passages(
    connection: &Checkout,
    query: &ParsedQuery,
    hits: &[(MessageId, Vec<Source>)],
    first_line: FirstLine,
) -> Vec<(MessageId, Vec<Match>)> {
    // What a body matched by: the words, as the index matched them.
    let terms: Vec<String> = query
        .searchable_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.clone())
        .filter(|value| !value.is_empty())
        .collect();
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

/// A body told apart into the person's own words and the history they
/// quoted (D6), and whether the text opens with either.
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
                    // "On Fri, Ada wrote:" says who wrote the quote under it:
                    // neither the person's words nor the history (the line
                    // the reader sets apart as the attribution).
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

    /// The person's own words matched, with their passage.
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

    /// The quoted history matched, with its passage.
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

/// Whether `terms` match anywhere in `words`, as the highlighter finds them.
fn found(words: &str, terms: &[String]) -> bool {
    !postio_search::highlight::find(words, terms).is_empty()
}

/// A body's matches: where in the person's own words, and where in what
/// they quoted, each with its passage.
fn body_matches(text: &str, terms: &[String], first_line: FirstLine) -> Vec<Match> {
    // The message's first line is the row's preview (D7): never the
    // passage, in whichever kind of stretch it falls -- unless it was an
    // attribution, which neither kind keeps.
    let told = Told::of(text);
    let mut matches: Vec<Match> = told
        .own_match(terms, first_line)
        .into_iter()
        .chain(told.quoted_match(terms, first_line))
        .collect();
    if matches.is_empty() {
        // The index matched what the highlighter cannot point at -- a
        // stem, a fold. Still the body; nothing to cut.
        matches.push(Match {
            source: Source::Body,
            passage: None,
            when: None,
        });
    }
    matches
}

/// How many of a conversation's messages Quick Look reads, newest kept: a
/// body read each, and a conversation of hundreds is read in its window.
const MATCHED_MESSAGES: usize = 50;

/// Every match in one conversation, for Quick Look (spec 010 US4,
/// FR-028): each message's own words, oldest first, with who wrote them
/// and when; its quoted history when the conversation does not hold what
/// it quotes; the file names that match; and the subject, once, last.
///
/// # A quote is said once
///
/// A reply quotes the message before it, and that message is in the
/// conversation with its own card: the quote would be the same words a
/// second time. A quoted passage found in another message's own words is
/// left out, and one that is not -- the conversation never held the mail
/// it quotes -- is the only place those words are, and stays.
///
/// # Where, not whether
///
/// The words are found where the highlighter finds them, as the rows'
/// passages are, over every message of the conversation the query found;
/// the query's other terms (`from:`, dates) chose the conversation, not
/// which of its messages are shown. The passages are cut as the results'
/// rows cut theirs ([`FirstLine::Avoided`]): around a match, never the
/// line that introduces a quote.
///
/// Empty when the conversation could not be read: Quick Look keeps what
/// the row already knew.
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

    // The words, as the index matched a body by; a subject and a file name
    // also by their own operators, as the executor tags them.
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
                // Every value of a set (spec 010, D26).
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

    let mut found = Vec::new();
    for (at, member) in members.iter().enumerate() {
        let when = Some(member.received_at);
        if let Some(body) = &told[at] {
            if let Some(mut own) = body.own_match(&words, FirstLine::Avoided) {
                own.when = when;
                found.push(ConversationMatch {
                    message: Some(member.id),
                    from: member.from.clone(),
                    found: own,
                });
            }
            let quoted = body
                .quoted_match(&words, FirstLine::Avoided)
                .filter(|quoted| {
                    !told.iter().enumerate().any(|(other, body)| {
                        other != at && body.as_ref().is_some_and(|body| said_in(quoted, &body.own))
                    })
                });
            if let Some(quoted) = quoted {
                found.push(ConversationMatch {
                    message: Some(member.id),
                    from: None,
                    found: quoted,
                });
            }
        }
        for (_, attachment, name) in files.iter().filter(|(of, _, _)| *of == member.id) {
            let marks = postio_search::highlight::find(name, &file_terms);
            if marks.is_empty() {
                continue;
            }
            found.push(ConversationMatch {
                message: Some(member.id),
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
    }
    let subject = members.iter().find_map(|member| {
        let subject = member.subject.as_deref()?;
        let marks = postio_search::highlight::find(subject, &subject_terms);
        (!marks.is_empty()).then(|| whole(subject, marks))
    });
    if let Some(subject) = subject {
        found.push(ConversationMatch {
            message: None,
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

/// Whether a quoted match's words are among `own`, a message's own words:
/// the quote is of that message. Compared with case and spacing folded,
/// because a quote rewraps the lines it quotes.
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

/// `text` whole as a passage, `marks` highlighted: a subject, a file name.
fn whole(text: &str, marks: Vec<std::ops::Range<usize>>) -> postio_search::results::Passage {
    postio_search::results::Passage {
        text: text.to_owned(),
        ranges: marks,
        elided_start: false,
        elided_end: false,
    }
}

/// The named attachments of `messages`: each one's message, id and name.
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

/// Quoted lines without their `>` markers: what was said, not how it was
/// quoted.
fn unquoted(lines: &str) -> String {
    lines
        .split_inclusive('\n')
        .map(|line| line.trim_start_matches(['>', ' ', '\t']))
        .collect()
}

/// What `prefix` could become (spec 010 US7): the words, labels, lists and
/// files it begins, or `field`'s values, each counted. `None` when the
/// index could not be read.
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

/// The looser searches a query that found nothing offers, each with the
/// conversations it would find: those that would find none left out, most
/// first, ties in the query's own order (spec 010 US6, FR-043).
///
/// `None` when the counts could not be taken.
pub async fn relaxations(
    connection: &Checkout,
    account: AccountScope,
    query: &ParsedQuery,
    today: chrono::NaiveDate,
) -> Option<Vec<(postio_search::relax::Relaxation, u64)>> {
    let offered = postio_search::relax::relax(query);
    let counts = postio_index::executor::relaxation_counts(connection, account, &offered, today)
        .await
        .map_err(|error| tracing::warn!(%error, "the relaxations could not be counted"))
        .ok()?;
    let mut counted: Vec<_> = offered
        .into_iter()
        .zip(counts)
        .filter(|(_, count)| *count > 0)
        .collect();
    counted.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    Some(counted)
}
