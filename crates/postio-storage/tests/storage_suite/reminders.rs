//! Focus's reminders, as the store keeps them (spec 007 US5, research R7):
//! one standing reminder per conversation, fired by the due timer when
//! nobody replied, cancelled or settled when somebody did.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::{EmailAddress, Message, RfcMessageId};
use postio_storage::Connection;
use postio_storage::repository::{MessageRepository, ReminderRepository, ThreadingRepository};
use postio_storage::test_support;

fn at(hour: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 29, 0, 0, 0).unwrap() + TimeDelta::hours(hour)
}

/// Files a message from `sender` and threads it, as a sync pass does.
async fn file(
    connection: &Connection,
    (account, mailbox): (AccountId, MailboxId),
    hour: i64,
    rfc: &str,
    references: &[&str],
    sender: &str,
) -> (MessageId, ThreadId) {
    let mut message = Message::new(account, mailbox, at(hour));
    message.rfc_message_id = Some(RfcMessageId::new(rfc));
    message.references = references.iter().map(RfcMessageId::new).collect();
    message.from = vec![EmailAddress::new(None::<String>, sender)];
    let id = MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    let threaded = ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("thread");
    (id, threaded.thread_id)
}

#[tokio::test]
async fn setting_a_second_reminder_replaces_the_first_and_clearing_takes_it_away() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let (first, thread) = file(
        &connection,
        (account.id, inbox),
        0,
        "a@x.test",
        &[],
        "ada@example.com",
    )
    .await;
    let (second, _) = file(
        &connection,
        (account.id, inbox),
        1,
        "b@x.test",
        &["a@x.test"],
        "me@example.com",
    )
    .await;
    let reminders = ReminderRepository::new(&connection);
    assert_eq!(reminders.standing(thread).await.expect("a read"), None);

    let id = reminders
        .set(thread, first, at(48), at(2))
        .await
        .expect("set");
    assert!(reminders.fire(id, at(49)).await.expect("fired"));
    assert!(
        reminders
            .standing(thread)
            .await
            .expect("a read")
            .expect("one")
            .is_surfaced(),
        "fired and standing: surfaced"
    );

    // Naming a new time replaces the one standing, and a surfaced row goes
    // back to waiting.
    let again = reminders
        .set(thread, second, at(96), at(50))
        .await
        .expect("set");
    assert_eq!(again, id, "one reminder per conversation");
    let standing = reminders
        .standing(thread)
        .await
        .expect("a read")
        .expect("one");
    assert_eq!(standing.anchor, second);
    assert_eq!(standing.set_at, at(50));
    assert_eq!(standing.due_at, at(96));
    assert_eq!(standing.fired_at, None);
    assert!(!standing.is_surfaced());

    assert!(reminders.clear(thread).await.expect("cleared"));
    assert!(!reminders.clear(thread).await.expect("nothing left"));
    assert_eq!(reminders.standing(thread).await.expect("a read"), None);
}

#[tokio::test]
async fn a_reminder_falls_due_fires_once_and_a_reply_cancels_or_settles_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let (anchor_a, thread_a) = file(
        &connection,
        (account.id, inbox),
        0,
        "a@x.test",
        &[],
        "me@example.com",
    )
    .await;
    let (anchor_b, thread_b) = file(
        &connection,
        (account.id, inbox),
        1,
        "b@x.test",
        &[],
        "me@example.com",
    )
    .await;
    let (anchor_c, thread_c) = file(
        &connection,
        (account.id, inbox),
        2,
        "c@x.test",
        &[],
        "me@example.com",
    )
    .await;
    let reminders = ReminderRepository::new(&connection);
    let a = reminders
        .set(thread_a, anchor_a, at(10), at(3))
        .await
        .expect("set");
    let b = reminders
        .set(thread_b, anchor_b, at(20), at(3))
        .await
        .expect("set");
    let c = reminders
        .set(thread_c, anchor_c, at(30), at(3))
        .await
        .expect("set");

    assert!(reminders.due(at(9)).await.expect("a read").is_empty());
    let due: Vec<_> = reminders
        .due(at(25))
        .await
        .expect("a read")
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(due, vec![a, b], "due soonest first, and c is not due yet");

    // Nobody replied to a: it fires, once, and from then on it is surfaced.
    assert!(reminders.fire(a, at(25)).await.expect("fired"));
    assert!(!reminders.fire(a, at(26)).await.expect("already fired"));
    let due: Vec<_> = reminders
        .due(at(25))
        .await
        .expect("a read")
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(due, vec![b], "a fired reminder is not due again");
    assert_eq!(
        reminders
            .surfaced()
            .await
            .expect("a read")
            .into_iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        vec![a]
    );

    // Somebody replied to b before it fired: cancelled, and it never fires.
    assert!(reminders.cancel(b, at(15)).await.expect("cancelled"));
    assert!(
        !reminders
            .cancel(b, at(16))
            .await
            .expect("already cancelled")
    );
    assert!(!reminders.fire(b, at(25)).await.expect("not fired"));
    assert!(
        reminders
            .standing(thread_b)
            .await
            .expect("a read")
            .is_none()
    );
    assert!(
        !reminders.cancel(a, at(27)).await.expect("a write"),
        "a reminder that fired is settled, not cancelled"
    );

    // A reply after a surfaced: settled, and it leaves Focus's inbox.
    assert!(reminders.settle(a, at(28)).await.expect("settled"));
    assert!(!reminders.settle(a, at(29)).await.expect("already settled"));
    assert!(reminders.surfaced().await.expect("a read").is_empty());

    // The ones asked about by the filing pass are those still standing.
    let standing = reminders
        .standing_on(&[thread_a, thread_b, thread_c])
        .await
        .expect("a read");
    assert_eq!(standing.iter().map(|r| r.id).collect::<Vec<_>>(), vec![c]);
    assert!(reminders.standing_on(&[]).await.expect("a read").is_empty());
}

#[tokio::test]
async fn who_wrote_since_a_reminder_was_set_is_everyone_but_the_earlier_mail() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let (_, thread) = file(
        &connection,
        (account.id, inbox),
        0,
        "a@x.test",
        &[],
        "me@example.com",
    )
    .await;
    file(
        &connection,
        (account.id, inbox),
        5,
        "b@x.test",
        &["a@x.test"],
        "Ada@Example.com",
    )
    .await;
    file(
        &connection,
        (account.id, inbox),
        6,
        "c@x.test",
        &["a@x.test"],
        "ada@example.com",
    )
    .await;
    let reminders = ReminderRepository::new(&connection);

    let mut writers = reminders
        .writers_since(thread, at(2))
        .await
        .expect("a read");
    writers.sort();
    assert_eq!(
        writers,
        vec!["ada@example.com".to_owned()],
        "normalised, once each"
    );
    assert!(
        reminders
            .writers_since(thread, at(6))
            .await
            .expect("a read")
            .is_empty()
    );
}

#[tokio::test]
async fn surfaced_in_shows_only_fired_reminders_on_conversations_with_mail_in_those_inboxes() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let (in_inbox, thread_in) = file(
        &connection,
        (account.id, inbox),
        0,
        "a@x.test",
        &[],
        "me@example.com",
    )
    .await;
    let (in_archive, thread_out) = file(
        &connection,
        (account.id, archive),
        1,
        "b@x.test",
        &[],
        "me@example.com",
    )
    .await;
    let reminders = ReminderRepository::new(&connection);
    let waiting = reminders
        .set(thread_in, in_inbox, at(5), at(2))
        .await
        .expect("set");
    let archived = reminders
        .set(thread_out, in_archive, at(5), at(2))
        .await
        .expect("set");
    assert!(reminders.surfaced_in(&[]).await.expect("a read").is_empty());
    assert!(
        reminders
            .surfaced_in(&[inbox])
            .await
            .expect("a read")
            .is_empty(),
        "unfired reminders are not surfaced"
    );
    reminders.fire(waiting, at(6)).await.expect("fired");
    reminders.fire(archived, at(7)).await.expect("fired");

    let shown: Vec<_> = reminders
        .surfaced_in(&[inbox])
        .await
        .expect("a read")
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(
        shown,
        vec![waiting],
        "an archived conversation takes its row with it"
    );
    assert_eq!(reminders.surfaced().await.expect("a read").len(), 2);
}
