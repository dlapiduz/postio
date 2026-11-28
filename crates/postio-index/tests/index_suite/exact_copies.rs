use chrono::{Duration, TimeZone, Utc};
use postio_index::executor::{facets, relaxation_counts};
use postio_index::{SearchRequest, search};
use postio_model::{
    AccountScope, EmailAddress, Mailbox, MailboxId, MailboxRole, Message, MessageId, RfcMessageId,
};
use postio_search::relax::{Loosen, Relaxation};
use postio_search::results::{ConversationKey, ConversationOrder, Counted};
use postio_search::{ResultOrder, facets::Scope, parse};
use postio_storage::Connection;
use postio_storage::repository::{MailboxRepository, MessageRepository};

use crate::conversations::{conversations, now, store, today};

fn sent(account: &postio_model::Account, mailbox: MailboxId) -> Message {
    let mut message = Message::new(account.id, mailbox, now() - Duration::hours(3));
    message.rfc_message_id = Some(RfcMessageId::new("<survey-1@example.com>"));
    message.date = Some(Utc.with_ymd_and_hms(2026, 9, 30, 8, 59, 30).unwrap());
    message.size = 4_096;
    message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    message.to = vec![EmailAddress::new(Some("Grace"), "grace@example.com")];
    message.subject = Some("Nebula survey".to_owned());
    message
}

async fn filed(connection: &Connection, mut message: Message, body: Option<&str>) -> Message {
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("file the message");
    if let Some(body) = body {
        postio_index::index::index_body(connection, message.id.get(), Some(body))
            .await
            .expect("index its body");
    }
    message
}

async fn folder(connection: &Connection, account: &postio_model::Account, name: &str) -> MailboxId {
    let mut mailbox = Mailbox::new(account.id, name, Some('/'));
    mailbox.role = MailboxRole::Sent;
    MailboxRepository::new(connection)
        .create(&mut mailbox)
        .await
        .expect("a folder");
    mailbox.id
}

#[derive(Debug, PartialEq, Eq)]
struct Seen {
    hits: Vec<MessageId>,
    total: u64,
    scope: u64,
    conversations: Vec<ConversationKey>,
    conversation_total: u64,
    relaxed: u64,
}

async fn seen(connection: &Connection, account: &postio_model::Account, query: &str) -> Seen {
    let parsed = parse(query, today());
    let mut seen = None;
    for order in [ResultOrder::Relevance, ResultOrder::Newest] {
        let request = SearchRequest {
            mailboxes: None,
            account: AccountScope::Account(account.id),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 10,
            order,
        };
        let results = search(connection, &request, now()).await.expect("search");
        let mut hits: Vec<MessageId> = results.hits.iter().map(|hit| hit.message_id).collect();
        hits.sort();
        let scope = facets(connection, &request)
            .await
            .expect("facets")
            .scopes
            .iter()
            .find(|count| count.scope == Scope::AllMail)
            .expect("an All mail count")
            .hits;
        let found = conversations(connection, query, ConversationOrder::BestMatch, 0, 10).await;
        let relaxed = relaxation_counts(
            connection,
            AccountScope::Unified,
            &[Relaxation {
                loosen: Loosen::Drop { token: 0 },
                query: query.to_owned(),
            }],
            today(),
        )
        .await
        .expect("a relaxation count");
        let this = Seen {
            hits,
            total: results.total_hits,
            scope,
            conversations: {
                let mut keys: Vec<ConversationKey> = found.hits.iter().map(|hit| hit.key).collect();
                keys.sort_by_key(|key| match key {
                    ConversationKey::Lone(id) => id.get(),
                    ConversationKey::Thread(id) => -id.get(),
                });
                keys
            },
            conversation_total: found.total,
            relaxed: match relaxed.as_slice() {
                [Counted { conversations, .. }] => *conversations,
                other => panic!("one count for one query, got {other:?}"),
            },
        };
        match &seen {
            None => seen = Some(this),
            Some(first) => assert_eq!(first, &this, "{query:?}: the two orders disagree"),
        }
    }
    seen.expect("two orders")
}

fn once(message: &Message) -> Seen {
    Seen {
        hits: vec![message.id],
        total: 1,
        scope: 1,
        conversations: vec![ConversationKey::Lone(message.id)],
        conversation_total: 1,
        relaxed: 1,
    }
}

fn twice(first: &Message, second: &Message) -> Seen {
    Seen {
        hits: vec![first.id, second.id],
        total: 2,
        scope: 2,
        conversations: vec![
            ConversationKey::Lone(first.id),
            ConversationKey::Lone(second.id),
        ],
        conversation_total: 2,
        relaxed: 2,
    }
}

#[tokio::test]
async fn a_message_copied_into_a_second_folder_is_one_hit() {
    let (_database, connection, account, _inbox) = store().await;
    let sent_folder = folder(&connection, &account, "Sent").await;
    let sent_messages = folder(&connection, &account, "Sent Messages").await;
    let first = filed(
        &connection,
        sent(&account, sent_folder),
        Some("the nebula, mapped"),
    )
    .await;
    let mut copy = sent(&account, sent_messages);
    copy.received_at = first.received_at + Duration::minutes(5);
    let copy = filed(&connection, copy, Some("the nebula, mapped")).await;
    assert_ne!(first.id, copy.id);

    for query in [
        "nebula",
        "survey from:ada",
        "mapped",
        "from:ada",
        "is:unread",
    ] {
        assert_eq!(
            seen(&connection, &account, query).await,
            once(&first),
            "{query:?}"
        );
    }

    let parsed = parse("nebula", today());
    let refinements = facets(
        &connection,
        &SearchRequest {
            mailboxes: None,
            account: AccountScope::Account(account.id),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 10,
            order: ResultOrder::Relevance,
        },
    )
    .await
    .expect("facets")
    .refinements;
    for token in ["in:Sent", "in:\"Sent Messages\""] {
        assert_eq!(
            refinements
                .iter()
                .find(|refinement| refinement.token == token)
                .map(|refinement| refinement.hits),
            Some(1),
            "{token}: {refinements:#?}"
        );
    }

    let parsed = parse("nebula in:\"Sent Messages\"", today());
    let results = search(
        &connection,
        &SearchRequest {
            mailboxes: None,
            account: AccountScope::Account(account.id),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 10,
            order: ResultOrder::Relevance,
        },
        now(),
    )
    .await
    .expect("search");
    assert_eq!(
        results
            .hits
            .iter()
            .map(|hit| hit.message_id)
            .collect::<Vec<_>>(),
        [copy.id]
    );
}

#[tokio::test]
async fn a_copy_whose_body_is_not_here_yet_hides_nothing() {
    let (_database, connection, account, _inbox) = store().await;
    let sent_folder = folder(&connection, &account, "Sent").await;
    let sent_messages = folder(&connection, &account, "Sent Messages").await;
    let first = filed(&connection, sent(&account, sent_folder), None).await;
    let copy = filed(
        &connection,
        sent(&account, sent_messages),
        Some("the nebula, mapped"),
    )
    .await;

    assert_eq!(seen(&connection, &account, "mapped").await, once(&copy));
    assert_eq!(seen(&connection, &account, "survey").await, once(&first));
}

#[tokio::test]
async fn a_shared_message_id_alone_collapses_nothing() {
    for (what, differ) in [
        (
            "size",
            (|message: &mut Message| message.size += 1) as fn(&mut Message),
        ),
        ("date", |message: &mut Message| {
            message.date = message.date.map(|date| date + Duration::seconds(1));
        }),
        ("sender", |message: &mut Message| {
            message.from = vec![EmailAddress::new(Some("Mallory"), "mallory@example.com")];
        }),
    ] {
        let (_database, connection, account, _inbox) = store().await;
        let sent_folder = folder(&connection, &account, "Sent").await;
        let sent_messages = folder(&connection, &account, "Sent Messages").await;
        let first = filed(
            &connection,
            sent(&account, sent_folder),
            Some("the nebula, mapped"),
        )
        .await;
        let mut other = sent(&account, sent_messages);
        differ(&mut other);
        let other = filed(&connection, other, Some("the nebula, mapped")).await;
        for query in ["nebula", "mapped", "survey from:ada"] {
            let expected = if what == "sender" && query.contains("from:ada") {
                once(&first)
            } else {
                twice(&first, &other)
            };
            assert_eq!(
                seen(&connection, &account, query).await,
                expected,
                "{what}, {query:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_copy_costs_no_statement() {
    use postio_storage::test_support::counting::{Counts, counted_async, install};

    let mut measured: Vec<Vec<(&str, Counts)>> = Vec::new();
    for copies in [false, true] {
        let (_database, connection, account, _inbox) = store().await;
        let sent_folder = folder(&connection, &account, "Sent").await;
        let sent_messages = folder(&connection, &account, "Sent Messages").await;
        filed(
            &connection,
            sent(&account, sent_folder),
            Some("the nebula, mapped"),
        )
        .await;
        if copies {
            filed(
                &connection,
                sent(&account, sent_messages),
                Some("the nebula, mapped"),
            )
            .await;
        }
        install(&connection);
        let mut each = Vec::new();
        for query in ["nebula", "survey from:ada", "from:ada"] {
            let parsed = parse(query, today());
            let request = SearchRequest {
                mailboxes: None,
                account: AccountScope::Account(account.id),
                query: &parsed,
                scope: Scope::AllMail,
                limit: 10,
                order: ResultOrder::Relevance,
            };
            let counts = counted_async(async || {
                search(&connection, &request, now()).await.expect("search");
            })
            .await;
            each.push(("search", counts));
            let counts = counted_async(async || {
                conversations(&connection, query, ConversationOrder::BestMatch, 0, 10).await;
            })
            .await;
            each.push(("conversations", counts));
        }
        measured.push(each);
    }
    for ((path, once), (_, twice)) in measured[0].iter().zip(&measured[1]) {
        assert_eq!(once.statements, twice.statements, "{path}: statements");
        if *path == "search" {
            assert_eq!(once.rows, twice.rows, "{path}: rows");
        }
    }
}
