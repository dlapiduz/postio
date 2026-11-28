use chrono::{Duration, TimeZone, Utc};
use postio_model::{AccountScope, EmailAddress, Flag, MailboxId, Message, MessageId, Thread};
use postio_search::ResultOrder;
use postio_search::results::{GroupBy, StatsGroup};
use postio_session::search::{page, stats};
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_storage::test_support;

struct World {
    _database: test_support::TempStore,
    connection: postio_storage::Checkout,
    inbox: MailboxId,
    newest_first: Vec<MessageId>,
}

fn day(nth: i64) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap() + Duration::days(nth)
}

async fn world() -> World {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let messages = MessageRepository::new(&connection);
    let mut made = Vec::new();
    for (from, nth, mailbox, unread, flagged) in [
        ("ada@example.com", 0, inbox, false, true),
        ("ada@example.com", 1, inbox, true, false),
        ("ada@example.com", 40, inbox, false, false),
        ("grace@example.net", 2, inbox, true, false),
        ("test@example.com", 3, inbox, false, false),
        ("tomas@example.net", 4, archive, true, true),
    ] {
        let mut message = Message::new(account.id, mailbox, day(nth));
        message.from = vec![EmailAddress::new(None::<String>, from)];
        message.subject = Some("Harbour plan".to_owned());
        if !unread {
            message.flags.insert(Flag::Seen);
        }
        if flagged {
            message.flags.insert(Flag::Flagged);
        }
        made.push((
            nth,
            mailbox,
            messages.create(&mut message).await.expect("a message"),
        ));
    }
    let threads = ThreadRepository::new(&connection);
    let mut thread = Thread::new(account.id);
    let thread = threads.create(&mut thread).await.expect("a thread");
    for id in [made[1].2, made[4].2] {
        threads.add_message(thread, id).await.expect("join");
    }
    let mut in_inbox: Vec<_> = made
        .iter()
        .filter(|(_, mailbox, _)| *mailbox == inbox)
        .collect();
    in_inbox.sort_by_key(|(nth, _, _)| std::cmp::Reverse(*nth));
    World {
        newest_first: in_inbox.iter().map(|(_, _, id)| *id).collect(),
        _database: database,
        connection,
        inbox,
    }
}

#[tokio::test]
async fn pages_walk_the_shared_folders_newest_first_and_stop_at_the_end() {
    let world = world().await;
    let query = postio_search::parse("harbour", day(50).date_naive());
    let shared = [world.inbox];
    let first = page(
        &world.connection,
        AccountScope::Unified,
        &query,
        &shared,
        ResultOrder::Newest,
        0,
        3,
    )
    .await
    .expect("a page");
    assert_eq!(
        first.ids,
        world.newest_first[..3],
        "the newest three in the inbox"
    );
    assert_eq!(
        first.total, 5,
        "the archive's message is not counted: it is not shared"
    );
    assert_eq!(first.next, Some(3));
    let last = page(
        &world.connection,
        AccountScope::Unified,
        &query,
        &shared,
        ResultOrder::Newest,
        3,
        3,
    )
    .await
    .expect("a page");
    assert_eq!(last.ids, world.newest_first[3..]);
    assert_eq!(last.next, None, "the last page says so");

    let best = page(
        &world.connection,
        AccountScope::Unified,
        &query,
        &shared,
        ResultOrder::Relevance,
        3,
        2,
    )
    .await
    .expect("a page");
    assert_eq!(
        (best.ids.len(), best.next),
        (2, None),
        "best first is one page from the top"
    );
}

#[tokio::test]
async fn stats_group_the_shared_matches_and_leave_the_person_out() {
    let world = world().await;
    let shared = [world.inbox];
    let senders = stats(
        &world.connection,
        AccountScope::Unified,
        "harbour",
        &shared,
        GroupBy::Sender,
        10,
        day(50).date_naive(),
    )
    .await
    .expect("stats");
    assert_eq!(
        senders.groups,
        vec![
            StatsGroup {
                key: "ada@example.com".to_owned(),
                count: 3,
                first: day(0),
                last: day(40),
                unread: 1,
                flagged: 1,
                automated: 0,
                replied: 1,
            },
            StatsGroup {
                key: "grace@example.net".to_owned(),
                count: 1,
                first: day(2),
                last: day(2),
                unread: 1,
                flagged: 0,
                automated: 0,
                replied: 0,
            },
        ],
        "ada's second message is in the thread the person answered; the person's own message is \
         no sender group, and tomas is in a folder that is not shared"
    );
    assert_eq!(
        (senders.total, senders.counted, senders.capped),
        (5, 5, false)
    );

    let domains = stats(
        &world.connection,
        AccountScope::Unified,
        "harbour",
        &shared,
        GroupBy::Domain,
        10,
        day(50).date_naive(),
    )
    .await
    .expect("stats");
    let keys: Vec<(&str, u32)> = domains
        .groups
        .iter()
        .map(|g| (g.key.as_str(), g.count))
        .collect();
    assert_eq!(keys, vec![("example.com", 3), ("example.net", 1)]);

    let months = stats(
        &world.connection,
        AccountScope::Unified,
        "harbour",
        &shared,
        GroupBy::Month,
        1,
        day(50).date_naive(),
    )
    .await
    .expect("stats");
    let keys: Vec<(&str, u32)> = months
        .groups
        .iter()
        .map(|g| (g.key.as_str(), g.count))
        .collect();
    assert_eq!(
        keys,
        vec![("2026-10", 4)],
        "a month counts the person's own mail too; one group asked for"
    );
}

#[tokio::test]
async fn stats_cost_the_same_statements_whatever_the_folder_holds() {
    use postio_storage::test_support::counting::{Counts, counted_async, install};
    let mut costs = Vec::new();
    for size in [10_i64, 120] {
        let database = test_support::temp().await;
        let connection = database.connect().await.expect("checkout");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("schema");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let messages = MessageRepository::new(&connection);
        for nth in 0..size {
            let mut message = Message::new(account.id, inbox, day(nth % 60));
            message.from = vec![EmailAddress::new(
                None::<String>,
                format!("sender{}@example.com", nth % 7),
            )];
            message.subject = Some("Harbour plan".to_owned());
            messages.create(&mut message).await.expect("a message");
        }
        install(&connection);
        let mut groups = 0;
        let counts: Counts = counted_async(async || {
            groups = stats(
                &connection,
                AccountScope::Unified,
                "harbour",
                &[inbox],
                GroupBy::Sender,
                50,
                day(90).date_naive(),
            )
            .await
            .expect("stats")
            .groups
            .len();
        })
        .await;
        assert_eq!(groups, 7);
        costs.push(counts.statements);
    }
    assert_eq!(
        costs[0], costs[1],
        "statements over 10 messages, then 120: counting must not read once per message"
    );
}
