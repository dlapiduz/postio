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

const DIVIDING: &[&str] = &[
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
    "from:ada",
    "from:norwood",
    "from:example.org",
    "from:abuse.example.com",
    "from:\"Ada Norwood\"",
    "from:Françoise",
    "from:harbour",
    "list:harbour-dev.lists.example.org",
    "list:weekly.news.example.org",
    "list:news.larkspur.example.com",
    "list:harbour",
    "list:example.org",
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
    "-from:ada.norwood@example.com",
    "-list:harbour",
    "from:ada.norwood@example.com list:harbour",
    "from:quinn.abara@example.net -list:harbour",
    "from:example.com -from:ada.norwood@example.com",
    "from:{ada.norwood@example.com quinn.abara@example.net}",
    "from:{ada tanaka.yoko@jp.example orders@shop.example.org}",
    "-from:{ada.norwood@example.com quinn.abara@example.net}",
    "list:{harbour weekly.news.example.org}",
    "to:{ada.norwood@example.com ren}",
    "subject:{walkthrough receipt}",
    "filename:{invite.ics badge.png}",
    "from:{example.com example.org} -list:{harbour}",
    "from:{ada quinn.abara@example.net} -subject:{walkthrough zeppelin}",
];

const NOTHING: &[&str] = &[
    "from:nobody@example.com",
    "from:francoise",
    "list:nothing.example",
    "from:ad",
    "to:nobody@example.com",
    "subject:zeppelin",
    "filename:nothing.pdf",
    "from:{nobody@example.com francoise}",
    "subject:{zeppelin dirigible}",
];

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 26).expect("a real date")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 26, 16, 9, 0).unwrap()
}

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

async fn by_executor(
    connection: &Connection,
    corpus: &Corpus,
    query: &ParsedQuery,
) -> BTreeSet<MessageId> {
    search(
        connection,
        &SearchRequest {
            mailboxes: None,
            account: AccountScope::Account(corpus.account),
            query,
            scope: Scope::AllMail,
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
async fn a_full_address_finds_the_mail_from_it_and_nothing_sent_to_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let corpus = corpus(&connection).await;
    let from_quinn: BTreeSet<MessageId> = corpus
        .messages
        .iter()
        .filter(|(_, message)| {
            message
                .from
                .iter()
                .any(|from| from.address == "quinn.abara@example.net")
        })
        .map(|(_, message)| message.id)
        .collect();
    assert!(from_quinn.len() >= 5, "the corpus this test counts on");

    let text = "from:quinn.abara@example.net";
    let query = parse(text, today());
    let executor = by_executor(&connection, &corpus, &query).await;
    assert_eq!(
        executor,
        from_quinn,
        "the executor also found {:?}",
        names(&corpus, executor.difference(&from_quinn).copied())
    );
    assert_eq!(by_matcher(&corpus, &query, text), from_quinn);
}

#[tokio::test]
async fn every_query_asks_something_the_corpus_answers_both_ways() {
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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let corpus = corpus(&connection).await;

    for (name, message) in &corpus.messages {
        let stored = postio_storage::sql::one(
            &connection,
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

#[test]
fn the_matcher_refuses_label_and_has_action() {
    // Labels and open markers are written after a message is filed, when the matcher runs, so it
    // must refuse them rather than answer from a state that does not exist yet (D12).
    use postio_search::matcher::Unsupported;
    for (text, token) in [
        ("label:atlas", "label:atlas"),
        ("-label:atlas", "-label:atlas"),
        (r#"label:"Q3 close""#, r#"label:"Q3 close""#),
        ("has:action", "has:action"),
        ("from:ada@example.com has:action", "has:action"),
        ("label:atlas from:ada@example.com", "label:atlas"),
    ] {
        let refused = Matcher::new(&parse(text, today())).err();
        assert_eq!(
            refused,
            Some(Unsupported::Token(token.to_owned())),
            "{text:?} is refused, naming {token:?}"
        );
    }
}
