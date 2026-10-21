//! Auto-emptying Junk and Trash (#1932), through the pass a running Postio
//! calls: what is due is hidden and queued for the server, what is not is
//! left, and off does nothing at all.

use chrono::{Duration, TimeZone, Utc};
use postio_core::retention::AutoEmpty;
use postio_model::{Flag, MailboxRole, Message, MessageId, Operation};
use postio_storage::repository::{
    ExpiryRepository, MailboxRepository, MessageRepository, OperationQueueRepository,
};
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap()
}

struct World {
    database: postio_storage::Store,
    account: postio_model::Account,
    junk: postio_model::MailboxId,
    trash: postio_model::MailboxId,
}

async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = test_support::account_with_inbox(&connection).await;
    let mut ids = Vec::new();
    for (path, role) in [("Junk", MailboxRole::Junk), ("Trash", MailboxRole::Trash)] {
        let mut mailbox = test_support::mailbox(&connection, &account, path).await;
        mailbox.role = role;
        MailboxRepository::new(&connection)
            .update(&mailbox)
            .await
            .expect("role");
        ids.push(mailbox.id);
    }
    drop(connection);
    World {
        database,
        account,
        junk: ids[0],
        trash: ids[1],
    }
}

async fn file(
    world: &World,
    mailbox: postio_model::MailboxId,
    days: i64,
    flagged: bool,
) -> MessageId {
    let connection = world.database.connect().await.expect("checkout");
    let mut message = Message::new(world.account.id, mailbox, now() - Duration::days(days));
    if flagged {
        message.flags.insert(Flag::Flagged);
    }
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("filed")
}

async fn queued_purges(world: &World) -> usize {
    let connection = world.database.connect().await.expect("checkout");
    OperationQueueRepository::new(&connection)
        .pending(world.account.id, now())
        .await
        .expect("queue")
        .iter()
        .filter(|row| matches!(row.operation, Operation::Purge { .. }))
        .count()
}

#[tokio::test]
async fn off_does_nothing_and_touches_nothing() {
    let world = world().await;
    file(&world, world.trash, 900, false).await;
    let counts = counted_async(|| async {
        let report = postio_session::auto_empty::run(&world.database, AutoEmpty::default(), now())
            .await
            .expect("pass");
        assert_eq!(report.purged, 0);
    })
    .await;
    assert_eq!(counts.statements, 0, "off is not a query");
    assert_eq!(queued_purges(&world).await, 0);
}

#[tokio::test]
async fn due_mail_is_purged_and_flagged_or_young_mail_is_kept() {
    let world = world().await;
    let old_junk = file(&world, world.junk, 20, false).await;
    file(&world, world.junk, 3, false).await;
    file(&world, world.junk, 500, true).await;
    let old_trash = file(&world, world.trash, 40, false).await;
    file(&world, world.trash, 20, false).await;

    let policy = AutoEmpty {
        junk_days: 7,
        trash_days: 30,
    };
    let report = postio_session::auto_empty::run(&world.database, policy, now())
        .await
        .expect("pass");
    assert_eq!(report.purged, 2);
    assert_eq!(queued_purges(&world).await, 2);

    // What is left due is nothing: the two are hidden, the rest are not due.
    let connection = world.database.connect().await.expect("checkout");
    for (role, mailbox, gone) in [
        (MailboxRole::Junk, world.junk, old_junk),
        (MailboxRole::Trash, world.trash, old_trash),
    ] {
        let found = ExpiryRepository::new(&connection)
            .expired_in(world.account.id, role, now(), 100)
            .await
            .expect("read");
        assert!(
            found
                .messages
                .iter()
                .all(|(m, id)| !(*m == mailbox && *id == gone)),
            "{role:?} still offers the purged message"
        );
    }

    // And a second pass has nothing left to do.
    let again = postio_session::auto_empty::run(&world.database, policy, now())
        .await
        .expect("pass");
    assert_eq!(again.purged, 0);
}

#[tokio::test]
async fn a_big_folder_is_emptied_a_page_at_a_time() {
    let world = world().await;
    let total = postio_session::auto_empty::PAGE as usize + 5;
    let connection = world.database.connect().await.expect("checkout");
    for n in 0..total {
        let mut message = Message::new(world.account.id, world.trash, now() - Duration::days(60));
        message.subject = Some(format!("old {n}"));
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("filed");
    }
    drop(connection);
    let report = postio_session::auto_empty::run(
        &world.database,
        AutoEmpty {
            junk_days: 0,
            trash_days: 30,
        },
        now(),
    )
    .await
    .expect("pass");
    assert_eq!(report.purged, total, "two pages, one pass");
    assert_eq!(queued_purges(&world).await, total);
}
