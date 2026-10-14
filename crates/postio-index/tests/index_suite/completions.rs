//! `executor::completions` (spec 010, US7, FR-042): what a prefix could
//! become, each with what it would find.
//!
//! The index has no term dictionary (research R1), so the words come from
//! the best documents of a `prefix*` match, metadata first, as
//! `words_near` recovers them (D22); bodies are read only when the
//! metadata says fewer than three words. Each suggestion's count is the
//! conversation total its query gives (US7's independent test), so each is
//! checked against `search_conversations` asked that query. People for
//! `from:` and `to:` are ranked two-way (D21): they wrote to you plus you
//! wrote to them, ties by the latest either way.

use chrono::Duration;
use postio_index::executor::completions;
use postio_model::{AccountScope, MessageId};
use postio_search::query::Field;
use postio_search::results::ConversationOrder;
use postio_storage::Connection;
use postio_storage::repository::LabelRepository;
use postio_storage::test_support::counting::{counted_async, install};

use crate::conversations::{Mail, conversations, file, now, store};

/// The conversations `query` finds: what a suggestion's count must say.
async fn total(connection: &Connection, query: &str) -> u64 {
    conversations(connection, query, ConversationOrder::Newest, 0, 1)
        .await
        .total
}

/// Gives `message` the mailing list `id`, as a list's mail carries it.
async fn on_list(connection: &Connection, message: MessageId, id: &str) {
    connection
        .execute(
            "UPDATE messages SET list_id = ?1 WHERE id = ?2",
            (id, message.get()),
        )
        .await
        .expect("list it");
}

#[tokio::test]
async fn a_prefix_offers_the_word_a_label_a_list_and_files_each_counted() {
    let (_database, connection, account, inbox) = store().await;
    let days = Duration::days;
    let mut atlas = postio_model::Label::new(account.id, "Atlas");
    let atlas = LabelRepository::new(&connection)
        .create(&mut atlas)
        .await
        .expect("a label");
    let mut ids: Vec<MessageId> = Vec::new();
    for (subject, file_name, ago) in [
        ("Atlas plan", Some("Atlas-Q3-budget.xlsx"), days(1)),
        ("Re: Atlas plan", None, days(2)),
        ("Atlas staffing", None, days(3)),
        ("Atlas headcount", Some("attendance.pdf"), days(4)),
        ("Atlantic crossing", None, days(5)),
    ] {
        let message = file(
            &connection,
            &account,
            inbox,
            Mail {
                subject,
                body: "notes for the quarter",
                file: file_name,
                ago,
                ..Mail::default()
            },
        )
        .await;
        ids.push(message.id);
    }
    // Two carry the label, one came through the list.
    for id in &ids[..2] {
        LabelRepository::new(&connection)
            .attach(*id, atlas)
            .await
            .expect("label it");
    }
    on_list(&connection, ids[2], "atlas-planning.example.org").await;
    // Nothing else begins with "at", and some mail says nothing of it.
    file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Lunch",
            body: "soup",
            ago: days(6),
            ..Mail::default()
        },
    )
    .await;

    install(&connection);
    let mut found = None;
    let cost = counted_async(async || {
        found = Some(
            completions(&connection, AccountScope::Unified, "at", None)
                .await
                .expect("completions"),
        );
    })
    .await;
    let found = found.expect("answered");

    assert_eq!(
        found.ghost.as_deref(),
        Some("las"),
        "the rest of the commonest word: at|las"
    );
    let word = found.words.first().expect("a word");
    assert_eq!(word.text, "atlas");
    assert_eq!(word.query, "atlas");
    assert_eq!(word.count, total(&connection, "atlas").await);
    assert!(word.count >= 4, "the four Atlas threads: {word:?}");

    let label = found
        .labels
        .iter()
        .find(|label| label.text == "Atlas")
        .unwrap_or_else(|| panic!("the label: {:?}", found.labels));
    assert_eq!(label.query, "label:Atlas");
    assert_eq!(label.count, total(&connection, "label:Atlas").await);
    assert_eq!(label.count, 2);

    let list = found.lists.first().expect("the list");
    assert_eq!(list.text, "atlas-planning.example.org");
    assert_eq!(list.count, total(&connection, &list.query).await);
    assert_eq!(list.count, 1);

    let mut files: Vec<&str> = found.files.iter().map(|file| file.text.as_str()).collect();
    files.sort_unstable();
    assert_eq!(
        files,
        ["Atlas-Q3-budget.xlsx", "attendance.pdf"],
        "the files whose names hold a word beginning with the prefix"
    );

    assert!(
        cost.statements <= 4,
        "{} statements: the documents, the labels and one count of them all",
        cost.statements
    );
}

#[tokio::test]
async fn bodies_are_read_only_when_the_metadata_says_fewer_than_three_words() {
    let (_database, connection, account, inbox) = store().await;
    let days = Duration::days;
    // Three words beginning "har" in subjects, and one more only a body says.
    for (subject, body, ago) in [
        ("Harbor review", "plain text", days(1)),
        ("Hardware order", "plain text", days(2)),
        ("Harvest dinner", "the harpsichord arrives", days(3)),
        ("Zoning", "the zeppelin lands", days(4)),
    ] {
        file(
            &connection,
            &account,
            inbox,
            Mail {
                subject,
                body,
                ago,
                ..Mail::default()
            },
        )
        .await;
    }

    install(&connection);
    let mut found = None;
    let metadata_only = counted_async(async || {
        found = Some(
            completions(&connection, AccountScope::Unified, "har", None)
                .await
                .expect("completions"),
        );
    })
    .await;
    let found = found.expect("answered");
    assert!(
        found.words.iter().all(|word| word.text != "harpsichord"),
        "three words from the metadata: the bodies are not read: {:?}",
        found.words
    );

    let mut found = None;
    let with_bodies = counted_async(async || {
        found = Some(
            completions(&connection, AccountScope::Unified, "zep", None)
                .await
                .expect("completions"),
        );
    })
    .await;
    let found = found.expect("answered");
    assert_eq!(
        found.words.first().map(|word| word.text.as_str()),
        Some("zeppelin"),
        "nothing in the metadata begins so: the bodies are"
    );
    assert_eq!(found.ghost.as_deref(), Some("pelin"));
    assert_eq!(
        with_bodies.statements,
        metadata_only.statements + 1,
        "one statement more, the bodies'"
    );
    assert!(with_bodies.statements <= 4);
}

#[tokio::test]
async fn from_offers_people_ranked_by_how_often_you_write_to_each_other() {
    let (_database, connection, _account, _inbox) = store().await;
    let at = |days: i64| (now() - Duration::days(days)).timestamp_millis();
    // (address, name, they wrote, you wrote, last either way in days ago)
    let people = [
        ("ada@example.com", Some("Ada Moreno"), 5, 0, 1),
        ("adam@example.net", Some("Adam Okoro"), 1, 6, 9),
        ("adele@example.org", None, 3, 2, 0),
        ("bob@example.com", Some("Bob Adeyemi"), 2, 2, 3),
        ("hidden@example.com", Some("Adrian Hidden"), 40, 0, 1),
        ("zed@example.com", Some("Zed"), 50, 50, 1),
    ];
    for (address, name, seen, sent, ago) in people {
        connection
            .execute(
                "INSERT INTO contacts (account_id, name, address, address_normalized,
                                       times_seen, last_seen_at, suppressed)
                 VALUES (NULL, ?1, ?2, ?2, ?3, ?4, ?5)",
                (
                    name,
                    address,
                    seen,
                    at(ago + 1),
                    i64::from(address.starts_with("hidden")),
                ),
            )
            .await
            .expect("a contact");
        if sent > 0 {
            connection
                .execute(
                    "INSERT INTO addresses (address, address_normalized) VALUES (?1, ?1)",
                    [address],
                )
                .await
                .expect("an address");
            connection
                .execute(
                    "INSERT INTO correspondents (address_id, sent_count, last_sent_at)
                     SELECT id, ?2, ?3 FROM addresses WHERE address_normalized = ?1",
                    (address, sent, at(ago)),
                )
                .await
                .expect("a correspondent");
        }
    }

    install(&connection);
    let mut found = None;
    let cost = counted_async(async || {
        found = Some(
            completions(&connection, AccountScope::Unified, "ad", Some(Field::From))
                .await
                .expect("completions"),
        );
    })
    .await;
    let found = found.expect("answered");
    let order: Vec<&str> = found
        .people
        .iter()
        .map(|person| person.address.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "adam@example.net",
            "adele@example.org",
            "ada@example.com",
            "bob@example.com",
        ],
        "seven, then two fives with the later first, then Bob, whose name \
         begins a word with ad; a suppressed contact and Zed are not offered"
    );
    let adam = &found.people[0];
    assert_eq!(adam.name.as_deref(), Some("Adam Okoro"));
    assert_eq!((adam.received, adam.sent), (1, 6));
    assert_eq!(
        adam.last.map(|last| last.timestamp_millis()),
        Some(at(9)),
        "the later of the two ways"
    );
    assert!(found.words.is_empty() && found.labels.is_empty());
    assert!(cost.statements <= 4, "{} statements", cost.statements);

    // Nothing typed yet: everyone, most written-to first.
    let everyone = completions(&connection, AccountScope::Unified, "", Some(Field::To))
        .await
        .expect("completions");
    assert_eq!(
        everyone
            .people
            .first()
            .map(|person| person.address.as_str()),
        Some("zed@example.com")
    );
}

#[tokio::test]
async fn label_and_in_offer_labels_and_folders_with_their_counts() {
    let (_database, connection, account, inbox) = store().await;
    let mut atlas = postio_model::Label::new(account.id, "Atlas");
    let atlas = LabelRepository::new(&connection)
        .create(&mut atlas)
        .await
        .expect("a label");
    let mut other = postio_model::Label::new(account.id, "Harbor");
    LabelRepository::new(&connection)
        .create(&mut other)
        .await
        .expect("a label");
    let message = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Plan",
            body: "words",
            ..Mail::default()
        },
    )
    .await;
    LabelRepository::new(&connection)
        .attach(message.id, atlas)
        .await
        .expect("label it");

    let labels = completions(&connection, AccountScope::Unified, "at", Some(Field::Label))
        .await
        .expect("completions");
    let named: Vec<(&str, &str, u64)> = labels
        .labels
        .iter()
        .map(|label| (label.text.as_str(), label.query.as_str(), label.count))
        .collect();
    assert_eq!(named, [("Atlas", "label:Atlas", 1)]);

    let folders = completions(&connection, AccountScope::Unified, "in", Some(Field::In))
        .await
        .expect("completions");
    let inbox_row = folders
        .folders
        .iter()
        .find(|folder| folder.text == "INBOX" || folder.text == "Inbox")
        .unwrap_or_else(|| panic!("the inbox: {:?}", folders.folders));
    assert_eq!(
        inbox_row.count,
        total(&connection, &inbox_row.query).await,
        "what in: that folder finds"
    );
}

#[tokio::test]
async fn a_completion_count_stops_at_its_cap_and_says_it_is_a_floor() {
    // D29 (maintainer, 2026-10-09): a suggestion as common as `as` is not
    // counted to the end of the mailbox. Three times the cap of messages
    // carry one label, each its own conversation; the count walks no
    // further than the cap needs and says it stopped.
    use postio_index::executor::COMPLETION_COUNT_CAP;
    let (_database, connection, account, inbox) = store().await;
    let mut many = postio_model::Label::new(account.id, "Atlas");
    let many = LabelRepository::new(&connection)
        .create(&mut many)
        .await
        .expect("a label");
    let messages = 3 * COMPLETION_COUNT_CAP;
    let ids = format!(
        "[{}]",
        (0..messages)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let received = now().timestamp_millis() - 86_400_000;
    // An operator's count walks `messages` and the label's set alone, so
    // the metadata index stands aside for the load.
    connection.execute("BEGIN", ()).await.expect("begin");
    postio_index::index::defer_documents(&connection)
        .await
        .expect("defer the index");
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, seen)
             SELECT ?1, ?2, ?3 - value, ?3 - value, 1 FROM json_each(?4)",
            (account.id.get(), inbox.get(), received, ids),
        )
        .await
        .expect("a mailbox past the cap");
    connection
        .execute(
            "INSERT INTO message_labels (message_id, label_id) SELECT id, ?1 FROM messages",
            (many.get(),),
        )
        .await
        .expect("all of it labelled");
    connection
        .execute("DELETE FROM search_documents_deferred", ())
        .await
        .expect("end the deferral");
    connection.execute("COMMIT", ()).await.expect("commit");

    install(&connection);
    let mut found = None;
    let cost = counted_async(async || {
        found = Some(
            completions(&connection, AccountScope::Unified, "at", Some(Field::Label))
                .await
                .expect("completions"),
        );
    })
    .await;
    let found = found.expect("answered");
    let label = found.labels.first().expect("the label");
    assert_eq!(label.text, "Atlas");
    assert_eq!(label.count, COMPLETION_COUNT_CAP, "the cap: {label:?}");
    assert!(label.capped, "a count that stopped says so: {label:?}");
    assert!(
        cost.rows <= 2 * COMPLETION_COUNT_CAP as usize + 8,
        "{} rows read for {messages} messages: the walk stops at the cap",
        cost.rows
    );

    // Under the cap, the count is exact and says so.
    connection
        .execute(
            "DELETE FROM message_labels WHERE message_id > (SELECT min(id) + 9 FROM messages)",
            (),
        )
        .await
        .expect("ten keep the label");
    let found = completions(&connection, AccountScope::Unified, "at", Some(Field::Label))
        .await
        .expect("completions");
    let label = found.labels.first().expect("the label");
    assert_eq!((label.count, label.capped), (10, false));
}
