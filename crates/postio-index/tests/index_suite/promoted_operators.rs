//! `is:bulk` and `is:automated` (spec 007, research R8): the operators of
//! the three promoted headers, answered from `messages.unsubscribe_offered`
//! and `messages.automation`, over the `.eml` corpus.

use std::collections::BTreeSet;

use chrono::{DateTime, TimeZone, Utc};
use postio_index::index::{ensure_schema, index_headers};
use postio_index::{SearchRequest, search};
use postio_model::{AccountId, AccountScope, Message, MessageId, test_corpus};
use postio_search::facets::Scope;
use postio_search::{ResultOrder, parse};
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 26, 16, 9, 0).unwrap()
}

/// Every fixture, parsed as sync would parse it, stored in one inbox.
async fn corpus(connection: &Connection) -> (AccountId, Vec<(&'static str, Message)>) {
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
    (account.id, messages)
}

async fn found(connection: &Connection, account: AccountId, text: &str) -> BTreeSet<MessageId> {
    let query = parse(text, now().date_naive());
    search(
        connection,
        &SearchRequest {
            account: AccountScope::Account(account),
            query: &query,
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

fn names(messages: &[(&'static str, Message)], ids: &BTreeSet<MessageId>) -> Vec<&'static str> {
    messages
        .iter()
        .filter(|(_, message)| ids.contains(&message.id))
        .map(|(name, _)| *name)
        .collect()
}

#[tokio::test]
async fn is_bulk_and_is_automated_find_what_the_corpus_s_headers_say() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, messages) = corpus(&connection).await;

    let bulk = names(&messages, &found(&connection, account, "is:bulk").await);
    let mut expected_bulk = vec![
        "html-designed-three-column",
        "html-newsletter",
        "html-newsletter-many-tables",
        "html-newsletter-own-page",
        "html-responsive-stacked-cells",
        "list-thread-01-root",
        "list-thread-02-reply",
        "list-thread-03-reply-sibling",
        "list-thread-04-reply-deep",
        "list-thread-05-reply-no-references",
        "list-thread-06-reply-subject-only",
        "list-thread-07-subject-change",
        "transactional-shipping-notice",
    ];
    let mut bulk = bulk;
    bulk.sort_unstable();
    expected_bulk.sort_unstable();
    assert_eq!(bulk, expected_bulk, "is:bulk");

    let mut automated = names(
        &messages,
        &found(&connection, account, "is:automated").await,
    );
    automated.sort_unstable();
    assert_eq!(
        automated,
        vec![
            "bounce-delivery-status",
            "invite-iana-zone",
            "invite-update-sequence",
        ],
        "is:automated"
    );

    // Negated, each is the rest of the corpus: a message that says none of
    // the three is known to say none.
    let everything = found(&connection, account, "").await.len();
    let not_bulk = found(&connection, account, "-is:bulk").await.len();
    assert_eq!(not_bulk + expected_bulk.len(), everything, "-is:bulk");
}

#[tokio::test]
async fn mail_a_first_sync_filed_answers_once_its_body_s_headers_arrive() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let fixture = test_corpus::all()
        .iter()
        .find(|fixture| fixture.name() == "html-newsletter")
        .expect("the newsletter fixture");
    let parsed = fixture.parse();
    // As a first sync files it: what the three headers say is not known.
    let mut message = fixture.parse();
    message.account_id = account.id;
    message.mailbox_id = inbox;
    message.promoted = None;
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("store");
    assert!(
        !found(&connection, account.id, "is:bulk")
            .await
            .contains(&id),
        "not known is not bulk"
    );

    // The body arrives, and its headers are indexed.
    index_headers(&connection, id.get(), &parsed.headers)
        .await
        .expect("indexed");
    assert!(
        found(&connection, account.id, "is:bulk")
            .await
            .contains(&id),
        "the body's headers said so"
    );
    assert_eq!(
        MessageRepository::new(&connection)
            .get(id)
            .await
            .expect("a read")
            .expect("the message")
            .promoted,
        parsed.promoted,
        "and the store keeps what they said"
    );
}
