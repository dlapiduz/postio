//! Focus's filter decisions (spec 007, data-model.md "`filter_decisions`"):
//! why a message was filed out of the inbox, from a fixed vocabulary the
//! store itself refuses to widen (FR-113).

use chrono::{DateTime, TimeZone, Utc};
use postio_model::{Message, MessageId};
use postio_storage::Connection;
use postio_storage::repository::{
    FilterDecision, FilterDecisionRepository, FilterLayer, FilterReason, MessageRepository,
};
use postio_storage::test_support;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 26, hour, 0, 0).unwrap()
}

async fn message(connection: &Connection) -> MessageId {
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    MessageRepository::new(connection)
        .create(&mut Message::new(account.id, inbox, at(8)))
        .await
        .expect("a message")
}

async fn insert(connection: &Connection, message: MessageId, reason: &str, layer: &str) -> bool {
    postio_storage::sql::execute(
        connection,
        "INSERT INTO filter_decisions (message_id, reason, layer, decided_at)
         VALUES (?1, ?2, ?3, 0)",
        vec![
            turso::Value::Integer(message.get()),
            turso::Value::Text(reason.to_owned()),
            turso::Value::Text(layer.to_owned()),
        ],
    )
    .await
    .is_ok()
}

#[tokio::test]
async fn the_store_refuses_a_reason_or_a_layer_it_has_no_word_for() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let message = message(&connection).await;
    let clear = async || {
        postio_storage::sql::execute(
            &connection,
            "DELETE FROM filter_decisions WHERE message_id = ?1",
            [message.get()],
        )
        .await
        .expect("cleared");
    };
    // The six reasons and four layers it has are stored: without this the
    // refusals below would pass against a store with no table at all.
    for reason in [
        "spam",
        "promotion",
        "notification",
        "receipt",
        "shipping",
        "social",
    ] {
        assert!(
            insert(&connection, message, reason, "header").await,
            "{reason}"
        );
        clear().await;
    }
    for layer in ["header", "senders", "server", "model"] {
        assert!(insert(&connection, message, "spam", layer).await, "{layer}");
        clear().await;
    }

    for reason in ["newsletter", "Spam", ""] {
        assert!(
            !insert(&connection, message, reason, "header").await,
            "the reason {reason:?} was stored"
        );
    }
    assert!(
        !insert(&connection, message, "spam", "correction").await,
        "a layer outside the vocabulary was stored"
    );
}

#[tokio::test]
async fn every_reason_and_layer_the_code_names_is_one_the_store_takes() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let message = message(&connection).await;
    let decisions = FilterDecisionRepository::new(&connection);
    for reason in FilterReason::ALL {
        for layer in FilterLayer::ALL {
            let decision = FilterDecision {
                message,
                reason,
                source: Some("Forge".to_owned()),
                layer,
                decided_at: at(9),
            };
            decisions.record(&decision).await.expect("recorded");
            assert_eq!(
                decisions.get(message).await.expect("a read"),
                Some(decision)
            );
        }
    }
}

#[tokio::test]
async fn a_decision_is_recorded_once_per_message_and_a_restore_deletes_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let message = message(&connection).await;
    let decisions = FilterDecisionRepository::new(&connection);
    assert_eq!(decisions.get(message).await.expect("a read"), None);

    let notification = FilterDecision {
        message,
        reason: FilterReason::Notification,
        source: Some("Forge".to_owned()),
        layer: FilterLayer::Senders,
        decided_at: at(9),
    };
    decisions.record(&notification).await.expect("recorded");
    // Deciding again is the latest word, not a second row: a message is
    // filtered for one reason.
    let spam = FilterDecision {
        reason: FilterReason::Spam,
        source: None,
        layer: FilterLayer::Server,
        decided_at: at(10),
        ..notification.clone()
    };
    decisions.record(&spam).await.expect("recorded again");
    assert_eq!(decisions.get(message).await.expect("a read"), Some(spam));

    // `R` deletes it; undo records it back, when it was decided included.
    assert!(decisions.delete(message).await.expect("restored"));
    assert_eq!(decisions.get(message).await.expect("a read"), None);
    assert!(
        !decisions.delete(message).await.expect("nothing to restore"),
        "a message with no decision has nothing to restore"
    );
    decisions.record(&notification).await.expect("undone");
    assert_eq!(
        decisions.get(message).await.expect("a read"),
        Some(notification)
    );
}

#[tokio::test]
async fn filtered_today_counts_the_decisions_since_midnight_in_one_statement() {
    // The empty inbox's "186 filtered today" (spec 007 screen 16): one
    // counted read, sought through `decided_at`'s index, never a walk.
    use postio_storage::test_support::counting::{counted_async, scans};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let decisions = FilterDecisionRepository::new(&connection);
    for hour in [2, 9, 11, 23] {
        let message = message(&connection).await;
        decisions
            .record(&FilterDecision {
                message,
                reason: FilterReason::Notification,
                source: None,
                layer: FilterLayer::Header,
                decided_at: at(hour),
            })
            .await
            .expect("recorded");
    }
    let midnight = at(8);
    let _ = decisions.count_since(midnight).await.expect("warm");
    let mut filtered = 0;
    let counts = counted_async(|| async {
        filtered = decisions.count_since(midnight).await.expect("counted");
    })
    .await;
    assert_eq!(filtered, 3, "the three decided since, not the one before");
    assert_eq!(counts.statements, 1, "one statement: {counts:?}");
    assert!(
        scans(&connection, FilterDecisionRepository::EXPLAIN_COUNT_SINCE)
            .await
            .is_empty(),
        "counted through the index on decided_at"
    );
}

#[tokio::test]
async fn filtered_mail_is_listed_newest_first_by_reason_and_counted_once() {
    // Screen 21, US9 scenarios 5 and 7: Filtered lists what is filtered and
    // not restored, newest first; a tab lists one reason; mail filtered
    // forty days ago is still there; and the tabs' counts are one
    // statement. Each read seeks its index and walks no table.
    use postio_storage::test_support::counting::{counted_async, scans};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let decisions = FilterDecisionRepository::new(&connection);
    let mut made = Vec::new();
    for (reason, decided) in [
        (FilterReason::Notification, at(9)),
        (FilterReason::Promotion, at(10)),
        (FilterReason::Notification, at(11)),
        (FilterReason::Spam, at(12)),
        (FilterReason::Receipt, at(8) - chrono::TimeDelta::days(40)),
        (FilterReason::Social, at(13)),
    ] {
        let id = MessageRepository::new(&connection)
            .create(&mut Message::new(account.id, inbox, decided))
            .await
            .expect("a message");
        decisions
            .record(&FilterDecision {
                message: id,
                reason,
                source: Some("Forge".to_owned()),
                layer: FilterLayer::Header,
                decided_at: decided,
            })
            .await
            .expect("recorded");
        made.push(id);
    }
    // The social one was restored: it is no longer filtered.
    decisions
        .restore(made[5], Some(at(14)))
        .await
        .expect("restored");

    let _ = decisions.tabs().await.expect("warm");
    let mut tabs = Vec::new();
    let counts = counted_async(|| async {
        tabs = decisions.tabs().await.expect("the tabs");
    })
    .await;
    assert_eq!(
        tabs,
        [
            (FilterReason::Spam, 1),
            (FilterReason::Promotion, 1),
            (FilterReason::Notification, 2),
            (FilterReason::Receipt, 1),
            (FilterReason::Shipping, 0),
            (FilterReason::Social, 0),
        ],
        "each reason's standing decisions, in the tabs' order"
    );
    assert_eq!(counts.statements, 1, "{counts:?}");

    let all: Vec<MessageId> = decisions
        .filtered(None, 0, 50)
        .await
        .expect("all")
        .iter()
        .map(|decision| decision.message)
        .collect();
    assert_eq!(
        all,
        [made[3], made[2], made[1], made[0], made[4]],
        "newest first, the forty-day-old one still there, the restored one gone"
    );
    let mut notifications = Vec::new();
    let counts = counted_async(|| async {
        notifications = decisions
            .filtered(Some(FilterReason::Notification), 0, 50)
            .await
            .expect("a tab");
    })
    .await;
    assert_eq!(
        notifications
            .iter()
            .map(|decision| decision.message)
            .collect::<Vec<_>>(),
        [made[2], made[0]],
        "only notifications"
    );
    assert_eq!(counts.statements, 1, "{counts:?}");
    assert_eq!(counts.rows, 2, "it reads what it lists: {counts:?}");
    for (label, sql) in [
        (
            "the tabs",
            FilterDecisionRepository::explain_tabs().to_owned(),
        ),
        ("all", FilterDecisionRepository::explain_filtered(false)),
        ("a tab", FilterDecisionRepository::explain_filtered(true)),
    ] {
        // A SELECT with no FROM of its own "scans" its one constant row.
        let walks: Vec<String> = scans(&connection, &sql)
            .await
            .into_iter()
            .filter(|step| step != "SCAN CONSTANT ROW")
            .collect();
        assert!(
            walks.is_empty(),
            "{label} walks a table:\n{}",
            test_support::plan(&connection, &sql).await
        );
    }
}
