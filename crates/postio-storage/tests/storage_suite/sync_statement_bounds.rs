//! What a sync's statements cost does not follow the size of the store (#1708).
//!
//! #1707 was a `SCAN messages` inside `upsert_batch`, paid on every batch of
//! a first sync. The instrument that finds the next one is not a row count --
//! a filter returns few rows however many it read -- but the planner: record
//! the SQL a piece of work issues, then ask for each statement's plan and
//! refuse any that reads all of a growing table, or all of one folder of it.

use chrono::{TimeZone, Utc};
use postio_model::{MailboxId, Message, RemoteId, RfcMessageId, Uid, UidValidity};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support::{self, counting};

fn at(seconds: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

fn from_server(mailbox: MailboxId, account: postio_model::AccountId, n: u32) -> Message {
    let mut message = Message::new(account, mailbox, at(i64::from(n)));
    message.rfc_message_id = Some(RfcMessageId::new(format!("<server-{n}@example.com>")));
    message.server.uid = Some(Uid::new(n));
    message.server.uid_validity = Some(UidValidity::new(7));
    message.server.remote_id = Some(RemoteId::new(format!("7:{n}")));
    message
}

/// Every statement in `issued` that reads all of a growing table or scope.
async fn offenders(
    connection: &postio_storage::Connection,
    issued: &std::collections::BTreeMap<String, usize>,
) -> Vec<String> {
    let mut found = Vec::new();
    for sql in issued.keys() {
        let steps = counting::unbounded(connection, sql, counting::GROWING_TABLES).await;
        if !steps.is_empty() {
            found.push(format!(
                "{steps:?}: {}",
                sql.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
    }
    found
}

#[tokio::test]
async fn an_upsert_batch_reads_no_whole_table() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut first: Vec<Message> = (1..=5).map(|n| from_server(inbox, account.id, n)).collect();
    messages
        .upsert_batch(&mut first)
        .await
        .expect("seed the store");

    counting::record();
    // Fresh messages and a changed one, so the insert and the update paths
    // both run.
    let mut batch: Vec<Message> = (1..=10)
        .map(|n| from_server(inbox, account.id, n))
        .collect();
    messages.upsert_batch(&mut batch).await.expect("the batch");
    let issued = counting::recorded();

    assert!(!issued.is_empty(), "nothing was recorded");
    let found = offenders(&connection, &issued).await;
    assert!(
        found.is_empty(),
        "a sync batch reads all of a table that grows with the store: {found:#?}"
    );
}

/// The backfill's top-up asks, after every few hundred bodies, for the newest
/// messages still missing one. Newest first means the ones already fetched
/// are at the front, so a query that walks the folder and filters walks more
/// of them each time: a hundred-thousand-message folder at 200 per top-up is
/// five hundred walks of an ever-longer prefix.
#[tokio::test]
async fn the_backfill_top_up_does_not_walk_the_bodies_it_already_has() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut batch: Vec<Message> = (1..=5).map(|n| from_server(inbox, account.id, n)).collect();
    messages
        .upsert_batch(&mut batch)
        .await
        .expect("seed the store");

    counting::record();
    messages
        .needing_backfill_from(inbox, 200, 0)
        .await
        .expect("candidates");
    let issued = counting::recorded();

    assert!(!issued.is_empty(), "nothing was recorded");
    let found = offenders(&connection, &issued).await;
    assert!(
        found.is_empty(),
        "the top-up reads every message of the folder to find the few it wants: {found:#?}"
    );
    for sql in issued.keys() {
        let plan = test_support::plan(&connection, sql).await;
        assert!(
            !test_support::sorts(&plan),
            "the top-up sorts instead of reading in index order: {plan}"
        );
    }
}

/// What the top-up answers is the same whichever way it finds it: the newest
/// first across both states that owe a body, the window and the offset
/// applied to the merged order, and a message in no state that owes one left
/// out.
#[tokio::test]
async fn the_top_up_answers_newest_first_across_both_states_owing_a_body() {
    use postio_model::BodyState;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let states = [
        BodyState::NotFetched,
        BodyState::HeadersOnly,
        BodyState::Full,
        BodyState::NotFetched,
        BodyState::HeadersOnly,
        BodyState::Partial,
        BodyState::HeadersOnly,
    ];
    let mut ids = Vec::new();
    for (n, state) in states.into_iter().enumerate() {
        let mut message = from_server(inbox, account.id, n as u32 + 1);
        message.sync.body_state = state;
        ids.push(messages.create(&mut message).await.expect("create"));
    }

    let ask = |limit, offset| {
        let messages = MessageRepository::new(&connection);
        async move {
            messages
                .needing_backfill_from(inbox, limit, offset)
                .await
                .expect("candidates")
                .into_iter()
                .map(|candidate| candidate.message_id)
                .collect::<Vec<_>>()
        }
    };
    // Newest is index 6; the Full (2) and Partial (5) are never offered.
    let all = [ids[6], ids[4], ids[3], ids[1], ids[0]];
    assert_eq!(ask(10, 0).await, all);
    assert_eq!(ask(2, 0).await, all[..2]);
    assert_eq!(ask(2, 2).await, all[2..4]);
    assert_eq!(ask(10, 4).await, all[4..]);
    assert_eq!(
        ask(u32::MAX, 0).await,
        all,
        "an unbounded window is not a bound"
    );
}
