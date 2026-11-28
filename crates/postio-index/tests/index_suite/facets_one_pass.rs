use chrono::{Duration, NaiveDate};
use postio_model::{Account, LabelId, MessageId};
use postio_search::facets::{SearchFacets, months_ending, preset_starts};
use postio_search::results::ConversationOrder;
use postio_storage::Connection;
use postio_storage::repository::{LabelRepository, Marker, MarkerRepository, MarkerSource};
use postio_storage::sql::{self, RowExt as _};
use postio_storage::test_support;
use postio_storage::test_support::counting;

use crate::conversations::{Mail, conversations, file, now, store, thread, today};

async fn label(connection: &Connection, account: &Account, name: &str) -> LabelId {
    let mut label = postio_model::Label::new(account.id, name);
    LabelRepository::new(connection)
        .create(&mut label)
        .await
        .expect("a label")
}

async fn mark(connection: &Connection, message: MessageId, dismissed: bool) {
    let markers = MarkerRepository::new(connection);
    markers
        .insert(&Marker {
            message,
            kind: postio_model::listing::MarkerKind::Question,
            source: MarkerSource::Detector,
            span: Some((0, 10)),
            excerpt: Some("Can you send it?".to_owned()),
            starts_at: None,
            ends_at: None,
            due_at: None,
            invite: None,
            invite_state: None,
            answer: None,
            dismissed_at: None,
        })
        .await
        .expect("a marker");
    if dismissed {
        markers
            .dismiss(message, Some(now()))
            .await
            .expect("dismiss it");
    }
}

async fn world() -> (postio_storage::Store, postio_storage::Checkout) {
    let (database, connection, account, inbox) = store().await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let atlas = label(&connection, &account, "Atlas").await;
    let q3 = label(&connection, &account, "Q3 close").await;
    let labels = LabelRepository::new(&connection);
    let days = Duration::days;
    let mail = |from, to, subject, body, ago| Mail {
        from,
        to,
        subject,
        body,
        ago,
        ..Mail::default()
    };

    let plan = file(
        &connection,
        &account,
        inbox,
        Mail {
            unread: true,
            file: Some("atlas.pdf"),
            ..mail("ada", &["bob"], "Atlas plan", "the atlas plan", days(2))
        },
    )
    .await;
    labels.attach(plan.id, atlas).await.expect("label it");
    let reply = file(
        &connection,
        &account,
        archive,
        mail("grace", &["bob", "carol"], "Re: plan", "re atlas", days(40)),
    )
    .await;
    let lunch = file(
        &connection,
        &account,
        inbox,
        mail("ada", &["bob"], "Re: plan", "lunch?", days(1)),
    )
    .await;
    thread(&connection, &account, &[plan.id, reply.id, lunch.id]).await;

    let budget = file(
        &connection,
        &account,
        archive,
        mail("grace", &["carol"], "Budget", "atlas numbers", days(100)),
    )
    .await;
    labels.attach(budget.id, q3).await.expect("label it");
    mark(&connection, budget.id, false).await;

    file(
        &connection,
        &account,
        inbox,
        mail("ada", &["bob"], "Atlas", "old atlas", days(400)),
    )
    .await;

    let late = file(
        &connection,
        &account,
        inbox,
        Mail {
            unread: true,
            ..mail("dan", &["carol"], "Atlas again", "atlas", days(10))
        },
    )
    .await;
    labels.attach(late.id, atlas).await.expect("label it");
    mark(&connection, late.id, true).await;

    file(
        &connection,
        &account,
        inbox,
        mail("ada", &["bob"], "Lunch", "pizza", days(3)),
    )
    .await;
    (database, connection)
}

async fn facets(connection: &Connection, query: &str) -> (u64, SearchFacets) {
    let results = conversations(connection, query, ConversationOrder::BestMatch, 0, 10).await;
    (results.total, results.facets)
}

async fn total(connection: &Connection, query: &str) -> u64 {
    conversations(connection, query, ConversationOrder::Newest, 0, 1)
        .await
        .total
}

async fn name(connection: &Connection, sql: &str, id: i64) -> String {
    sql::one(connection, sql, [id], |row| row.col::<String>(0))
        .await
        .expect("a name")
}

fn quoted(value: &str) -> String {
    if value.contains(' ') {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

#[tokio::test]
async fn every_facet_counts_what_its_term_would_find() {
    let (_database, connection) = world().await;
    let (total_hits, facets) = facets(&connection, "atlas").await;
    assert_eq!(total_hits, 4);
    assert!(!facets.capped);

    let address = "SELECT address FROM addresses WHERE id = ?1";
    let mut senders = Vec::new();
    for count in &facets.senders {
        senders.push((
            name(&connection, address, count.id.get()).await,
            count.conversations,
        ));
    }
    assert_eq!(
        senders,
        [
            ("ada@example.com".to_owned(), 2),
            ("grace@example.com".to_owned(), 2),
            ("dan@example.com".to_owned(), 1),
        ],
        "by count, most first; the thread counts for both its senders"
    );
    let mut recipients = Vec::new();
    for count in &facets.recipients {
        recipients.push((
            name(&connection, address, count.id.get()).await,
            count.conversations,
        ));
    }
    assert_eq!(
        recipients,
        [
            ("carol@example.com".to_owned(), 3),
            ("bob@example.com".to_owned(), 2),
        ]
    );
    let mut labels = Vec::new();
    for count in &facets.labels {
        labels.push((
            name(
                &connection,
                "SELECT name FROM labels WHERE id = ?1",
                count.id.get(),
            )
            .await,
            count.conversations,
        ));
    }
    assert_eq!(
        labels,
        [("Atlas".to_owned(), 2), ("Q3 close".to_owned(), 1)]
    );
    let mut folders = Vec::new();
    for count in &facets.folders {
        folders.push((
            name(
                &connection,
                "SELECT name FROM mailboxes WHERE id = ?1",
                count.id.get(),
            )
            .await,
            count.conversations,
        ));
    }
    assert_eq!(
        folders,
        [("INBOX".to_owned(), 3), ("Archive".to_owned(), 2)]
    );
    assert_eq!(facets.attachment, 1);
    assert_eq!(facets.action, 1, "a dismissed question is no action");
    assert_eq!(facets.unread, 2);

    let mut terms: Vec<(String, u64)> = Vec::new();
    for (address, count) in &senders {
        terms.push((format!("from:{address}"), *count));
    }
    for (address, count) in &recipients {
        terms.push((format!("to:{address}"), *count));
    }
    for (label, count) in &labels {
        terms.push((format!("label:{}", quoted(label)), *count));
    }
    for (folder, count) in &folders {
        terms.push((format!("in:{}", quoted(folder)), *count));
    }
    terms.push(("has:attachment".to_owned(), facets.attachment));
    terms.push(("has:action".to_owned(), facets.action));
    terms.push(("is:unread".to_owned(), facets.unread));
    for (term, count) in terms {
        assert_eq!(
            total(&connection, &format!("atlas {term}")).await,
            count,
            "the facet for {term} says {count}"
        );
    }
}

#[tokio::test]
async fn a_conversation_is_in_the_month_of_its_newest_match() {
    let (_database, connection) = world().await;
    let (_, facets) = facets(&connection, "atlas").await;

    let months: Vec<NaiveDate> = facets.months.iter().map(|month| month.month).collect();
    assert_eq!(months, months_ending(today()), "the last twelve months");
    let counts: Vec<u64> = facets
        .months
        .iter()
        .map(|month| month.conversations)
        .collect();
    assert_eq!(counts, [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 2]);
}

#[tokio::test]
async fn the_date_presets_count_what_their_after_would_find() {
    let (_database, connection) = world().await;
    let (total_hits, facets) = facets(&connection, "atlas").await;

    assert_eq!(facets.presets, [1, 2, 2, 3, 4]);
    assert_eq!(facets.presets[4], total_hits, "any time is everything");
    for (start, count) in preset_starts(today()).iter().zip(facets.presets) {
        let query = match start {
            Some(start) => format!("atlas after:{start}"),
            None => "atlas".to_owned(),
        };
        assert_eq!(
            total(&connection, &query).await,
            count,
            "the preset {query:?}"
        );
    }
}

#[tokio::test]
async fn a_match_past_the_cap_makes_every_count_a_floor() {
    let (_database, connection, account, inbox) = store().await;
    let cap = postio_search::results::CONVERSATION_WALK_CAP as i64;
    let ids = format!(
        "[{}]",
        (0..=cap)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let received = now().timestamp_millis() - 86_400_000;
    connection.execute("BEGIN", ()).await.expect("begin");
    postio_index::index::defer_documents(&connection)
        .await
        .expect("defer the index");
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, seen)
             SELECT ?1, ?2, ?3 - value, ?3 - value, 0 FROM json_each(?4)",
            (account.id.get(), inbox.get(), received, ids),
        )
        .await
        .expect("a mailbox past the cap");
    connection
        .execute("DELETE FROM search_documents_deferred", ())
        .await
        .expect("end the deferral");
    connection.execute("COMMIT", ()).await.expect("commit");

    let walk_cap = postio_search::results::CONVERSATION_WALK_CAP;
    assert_eq!(walk_cap, 5_000);
    counting::record();
    let results = conversations(&connection, "is:unread", ConversationOrder::Newest, 0, 4).await;
    assert!(results.capped);
    assert!(results.facets.capped, "the facets say they are floors too");
    assert_eq!(results.total, walk_cap);
    assert_eq!(results.facets.unread, walk_cap);
    assert_eq!(results.hits.len(), 4);
    let walked = counting::here();
    assert!(
        walked.rows as u64 <= 2 * walk_cap + 1_000,
        "the walk stops at the cap: {} rows read",
        walked.rows
    );

    let query = postio_search::parse("is:unread", today());
    let gtk = postio_index::search(
        &connection,
        &postio_index::SearchRequest {
            mailboxes: None,
            account: postio_model::AccountScope::Account(account.id),
            query: &query,
            scope: postio_search::facets::Scope::AllMail,
            limit: 4,
            order: postio_search::ResultOrder::Newest,
        },
        now(),
    )
    .await
    .expect("GTK's search");
    assert_eq!(gtk.total_hits, walk_cap + 1);
    assert!(!gtk.total_hits_capped);
}
