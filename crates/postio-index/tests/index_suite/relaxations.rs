//! `executor::relaxation_counts` (spec 010, US6): how many conversations
//! each way out of an empty search would find.
//!
//! Screen 13 offers a few looser searches, each with its count, and SC-008
//! asks that the count be what the search would show: so each count here
//! is checked against `search_conversations` asked the relaxed query. One
//! statement per variant, whatever the corpus (research R7).

use chrono::Duration;
use postio_index::executor::relaxation_counts;
use postio_model::AccountScope;
use postio_search::parse;
use postio_search::relax::{MAX_RELAXATIONS, relax};
use postio_search::results::ConversationOrder;
use postio_storage::repository::LabelRepository;
use postio_storage::test_support::counting::{counted_async, install};

use crate::conversations::{Mail, conversations, file, store, thread, today};

#[tokio::test]
async fn each_relaxation_counts_what_its_search_would_find() {
    let (_database, connection, account, inbox) = store().await;
    let mut harbor = postio_model::Label::new(account.id, "Harbor");
    let harbor = LabelRepository::new(&connection)
        .create(&mut harbor)
        .await
        .expect("a label");
    let days = Duration::days;
    // Atlas mail from Ada and Grace, some of it in a thread, one with a
    // file, one labelled; and nothing that says every word at once.
    let plan = file(
        &connection,
        &account,
        inbox,
        Mail {
            to: &["bob"],
            subject: "Atlas plan",
            body: "the atlas budget",
            file: Some("plan.pdf"),
            ago: days(3),
            ..Mail::default()
        },
    )
    .await;
    let reply = file(
        &connection,
        &account,
        inbox,
        Mail {
            from: "grace",
            to: &["ada"],
            subject: "Re: Atlas plan",
            body: "atlas, agreed",
            unread: true,
            ago: days(2),
            ..Mail::default()
        },
    )
    .await;
    thread(&connection, &account, &[plan.id, reply.id]).await;
    let labelled = file(
        &connection,
        &account,
        inbox,
        Mail {
            from: "grace",
            to: &["bob"],
            subject: "Harbor",
            body: "atlas at the harbor",
            ago: days(10),
            ..Mail::default()
        },
    )
    .await;
    LabelRepository::new(&connection)
        .attach(labelled.id, harbor)
        .await
        .expect("label it");
    file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Budget",
            body: "the budget, without the word",
            ago: days(20),
            ..Mail::default()
        },
    )
    .await;

    let query = parse(
        "atlas budget from:grace to:bob label:harbor has:attachment subject:plan -lunch",
        today(),
    );
    assert_eq!(
        conversations(&connection, query.input(), ConversationOrder::Newest, 0, 1)
            .await
            .total,
        0,
        "the query itself finds nothing"
    );
    let variants = relax(&query);
    assert_eq!(variants.len(), MAX_RELAXATIONS, "as many as there are");

    install(&connection);
    let mut counts = Vec::new();
    let cost = counted_async(async || {
        counts = relaxation_counts(&connection, AccountScope::Unified, &variants, today())
            .await
            .expect("the counts");
    })
    .await;
    assert_eq!(counts.len(), variants.len(), "one count per variant");
    assert!(
        cost.statements <= MAX_RELAXATIONS,
        "{} statements for {} variants: one each is research R7's budget",
        cost.statements,
        variants.len()
    );

    for (variant, count) in variants.iter().zip(&counts) {
        let expected = conversations(&connection, &variant.query, ConversationOrder::Newest, 0, 1)
            .await
            .total;
        assert_eq!(*count, expected, "the count for {:?}", variant.query);
    }

    // And over a corpus where something does match, the same holds for a
    // relaxation that finds several conversations, threaded and not.
    let broad = parse("atlas from:grace -harbor", today());
    let looser = relax(&broad);
    let counts = relaxation_counts(&connection, AccountScope::Unified, &looser, today())
        .await
        .expect("the counts");
    for (variant, count) in looser.iter().zip(&counts) {
        let expected = conversations(&connection, &variant.query, ConversationOrder::Newest, 0, 1)
            .await
            .total;
        assert_eq!(*count, expected, "the count for {:?}", variant.query);
    }
    assert!(
        counts.iter().any(|count| *count >= 2),
        "a relaxation here finds more than one conversation: {counts:?}"
    );
}
