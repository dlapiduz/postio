//! One language, two evaluators, and the test that keeps them one
//! (ADR 0008 Q1).
//!
//! A digest rule is a query in the one language (spec FR-127, research R13):
//! `from:<address>` in milestone 1, `list:` in milestone 2. It is asked of
//! mail as the sync pass files it, before anything is committed where SQL
//! could see it, so it is answered by `postio_search::matcher` in memory.
//! Its preview ("would have caught 9 messages in the last 90 days") is the
//! same query answered by the executor, over the store.
//!
//! Two evaluators of one language that disagree is the worst outcome
//! available: the preview would show one answer and the rule would do
//! another, and nothing else in the workspace could see it happen, because
//! every other test exercises one evaluator at a time. So this indexes the
//! whole `.eml` corpus, asks every query below of both, and asserts the
//! answers are identical, naming the fixtures either side disagrees about.
//!
//! The queries are the ones a digest rule is written in -- `from:` and
//! `list:`, and the query rules' `to:`, `subject:` and `filename:` -- chosen
//! so each
//! divides the corpus -- a query every message answers the same way agrees
//! trivially and proves nothing -- plus a few that should match nothing at
//! all, which is where a matcher too eager to say yes shows itself.

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use postio_index::index::ensure_schema;
use postio_index::{SearchRequest, search};
use postio_model::{AccountId, AccountScope, Message, MessageId, test_corpus};
use postio_search::facets::Scope;
use postio_search::matcher::{Document, Matcher};
use postio_search::{ParsedQuery, ResultOrder, parse};
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// Queries each of which some fixtures answer and others do not.
const DIVIDING: &[&str] = &[
    // Sender rules, as milestone 1 writes them.
    "from:ada.norwood@example.com",
    "from:weekly@news.example.org",
    "from:news@larkspur.example.com",
    "from:orders@shop.example.org",
    "from:orders@fernhill.example.org",
    "from:orders@shop.example.test",
    "from:pics@abuse.example.com",
    "from:no-reply@notify.example.org",
    "from:site-office@example.com",
    "from:MAILER-DAEMON@mx4.example.net",
    "from:tanaka.yoko@jp.example",
    "from:f.lemaitre@fr.example",
    "from:quinn.abara@example.net",
    "from:tove.bergstrom@example.com",
    // What a rule edited by hand may say instead: the language allows it.
    "from:ada",
    "from:norwood",
    "from:example.org",
    "from:abuse.example.com",
    "from:\"Ada Norwood\"",
    "from:Françoise",
    "from:harbour",
    // List rules, milestone 2's.
    "list:harbour-dev.lists.example.org",
    "list:weekly.news.example.org",
    "list:news.larkspur.example.com",
    "list:harbour",
    "list:example.org",
    // Query rules, milestone 2's: what is known of a message as it is
    // filed, before its body (spec 007 US14, T155).
    "to:ada.norwood@example.com",
    "to:quinn.abara@example.net",
    "to:harbour-dev@lists.example.org",
    "to:ren",
    "subject:walkthrough",
    "subject:\"Tide gate interlock\"",
    "subject:receipt",
    "subject:invitation",
    "filename:invite.ics",
    "filename:badge.png",
    "filename:ics",
    "-subject:walkthrough",
    "to:ada subject:walkthrough",
    "from:example.com -to:ada.norwood@example.com",
    // Negation, and more than one condition.
    "-from:ada.norwood@example.com",
    "-list:harbour",
    "from:ada.norwood@example.com list:harbour",
    "from:quinn.abara@example.net -list:harbour",
    "from:example.com -from:ada.norwood@example.com",
];

/// Queries no fixture should answer.
const NOTHING: &[&str] = &[
    "from:nobody@example.com",
    "from:francoise",
    "list:nothing.example",
    "from:ad",
    "to:nobody@example.com",
    "subject:zeppelin",
    "filename:nothing.pdf",
];

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 26).expect("a real date")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 26, 16, 9, 0).unwrap()
}

/// Every fixture, parsed as sync would parse it and stored in one inbox.
struct Corpus {
    account: AccountId,
    messages: Vec<(&'static str, Message)>,
}

async fn corpus(connection: &Connection) -> Corpus {
    ensure_schema(connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    let repository = MessageRepository::new(connection);
    let mut messages = Vec::new();
    for fixture in test_corpus::all() {
        let mut message = fixture.parse();
        message.account_id = account.id;
        message.mailbox_id = inbox;
        repository.create(&mut message).await.expect("store");
        messages.push((fixture.name(), message));
    }
    assert!(
        messages.len() > 40,
        "the corpus got smaller than this test's premise"
    );
    Corpus {
        account: account.id,
        messages,
    }
}

/// What the executor selects: the preview's answer.
async fn by_executor(
    connection: &Connection,
    corpus: &Corpus,
    query: &ParsedQuery,
) -> BTreeSet<MessageId> {
    search(
        connection,
        &SearchRequest {
            account: AccountScope::Account(corpus.account),
            query,
            scope: Scope::AllMail,
            // Past the corpus, so paging is never mistaken for disagreement.
            limit: 500,
            order: ResultOrder::Newest,
        },
        now(),
    )
    .await
    .expect("the search runs")
    .hits
    .into_iter()
    .map(|hit| hit.message_id)
    .collect()
}

/// What the matcher selects, one message at a time, as a rule would.
fn by_matcher(corpus: &Corpus, query: &ParsedQuery, text: &str) -> BTreeSet<MessageId> {
    let matcher = Matcher::new(query)
        .unwrap_or_else(|why| panic!("{text:?} is a digest rule's query, but: {why:?}"));
    corpus
        .messages
        .iter()
        .filter(|(_, message)| matcher.matches(message))
        .map(|(_, message)| message.id)
        .collect()
}

/// The fixtures behind a set of ids, for a failure a person can act on.
fn names(corpus: &Corpus, ids: impl IntoIterator<Item = MessageId>) -> Vec<&'static str> {
    let ids: BTreeSet<MessageId> = ids.into_iter().collect();
    corpus
        .messages
        .iter()
        .filter(|(_, message)| ids.contains(&message.id))
        .map(|(name, _)| *name)
        .collect()
}

#[tokio::test]
async fn the_matcher_and_the_executor_agree_on_every_digest_query() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let corpus = corpus(&connection).await;

    let mut disagreements = Vec::new();
    for text in DIVIDING.iter().chain(NOTHING) {
        let query = parse(text, today());
        let executor = by_executor(&connection, &corpus, &query).await;
        let matcher = by_matcher(&corpus, &query, text);
        if executor != matcher {
            disagreements.push(format!(
                "{text:?}\n    only the executor: {:?}\n    only the matcher:  {:?}",
                names(&corpus, executor.difference(&matcher).copied()),
                names(&corpus, matcher.difference(&executor).copied()),
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the two evaluators disagree on {} of {} queries:\n  {}",
        disagreements.len(),
        DIVIDING.len() + NOTHING.len(),
        disagreements.join("\n  ")
    );
}

#[tokio::test]
async fn every_query_asks_something_the_corpus_answers_both_ways() {
    // The premise of the test above: a query every fixture answers alike
    // agrees trivially, and one expected to match nothing must not match.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let corpus = corpus(&connection).await;
    let everything = corpus.messages.len();

    for text in DIVIDING {
        let selected = by_executor(&connection, &corpus, &parse(text, today())).await;
        assert!(
            !selected.is_empty() && selected.len() < everything,
            "{text:?} selects {} of {everything}, so it cannot tell two evaluators apart",
            selected.len()
        );
    }
    for text in NOTHING {
        let selected = by_executor(&connection, &corpus, &parse(text, today())).await;
        assert!(
            selected.is_empty(),
            "{text:?} was expected to select nothing, and selects {:?}",
            names(&corpus, selected)
        );
    }
}

#[tokio::test]
async fn the_matcher_reads_the_row_the_index_holds() {
    // What the matcher builds for a message is what the executor's triggers
    // wrote for it, column for column -- so a disagreement above is about
    // meaning, never about which text either side was looking at.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let corpus = corpus(&connection).await;

    for (name, message) in &corpus.messages {
        let stored = postio_storage::sql::one(
            &connection,
            // Keyed by the message's content (ADR 0046), not the occurrence.
            "SELECT sender, recipients, subject, filenames, list_id
               FROM search_documents
              WHERE content_id = (SELECT content_id FROM messages WHERE id = ?1)",
            [message.id.get()],
            |row| {
                use postio_storage::sql::RowExt as _;
                Ok(Document {
                    sender: row.col(0)?,
                    recipients: row.col(1)?,
                    subject: row.col(2)?,
                    filenames: row.col(3)?,
                    list_id: row.col(4)?,
                })
            },
        )
        .await
        .expect("every stored message has a row");
        assert_eq!(Document::of(message), stored, "{name}");
    }
}
