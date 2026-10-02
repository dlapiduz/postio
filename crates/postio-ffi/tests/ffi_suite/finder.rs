//! The search box's prefix modes, across the boundary: `#` folders,
//! `@` correspondents, `+` labels (`postio_ui::finder::MODES`).
//!
//! GTK's box has had all four since the finder was built; the Mac's had only
//! `>`, while its keyboard sheet -- read from the same `MODES` -- listed all
//! four. What crosses is each mode's matches, scored by the shared matcher,
//! and the sentence to show when there are none.

use chrono::Utc;
use postio_ffi::{ScopeFfi, Session, SessionOptions};
use postio_model::{EmailAddress, Label, Message};
use postio_storage::repository::{ContactRepository, LabelRepository, MessageRepository};
use postio_storage::test_support;

struct World {
    session: std::sync::Arc<Session>,
    database: postio_storage::Store,
    inbox: i64,
    message: i64,
    label: i64,
}

/// A store with an inbox, two more folders, a correspondent, a label and one
/// message, and a session over it with that message's folder open.
async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    test_support::mailbox(&connection, &account, "Receipts").await;
    test_support::mailbox(&connection, &account, "Wayland-devel").await;
    ContactRepository::new(&connection)
        .record(
            Some(account.id),
            &EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
            Utc::now(),
        )
        .await
        .expect("a correspondent");
    let mut label = Label::new(account.id, "Taxes");
    let label = LabelRepository::new(&connection)
        .create(&mut label)
        .await
        .expect("a label")
        .get();
    let mut message = Message::new(account.id, inbox, Utc::now());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("a message");

    let session = Session::open(SessionOptions::in_memory_with(database.clone()))
        .expect("a session over the store");
    session.open_scope(ScopeFfi::Mailbox {
        mailbox: inbox.get(),
    });
    let _ = session.row_at(0);
    session.settle_for_test();
    World {
        session,
        database,
        inbox: inbox.get(),
        message: message.id.get(),
        label,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_is_found_the_way_a_command_is() {
    // The palette's matcher, so `wd` finds `Wayland-devel` here exactly as
    // `cp` finds "Command palette" in `>`.
    let world = world().await;

    let found = world.session.finder_folders("wd".to_owned()).await;

    assert_eq!(
        found.hits.first().map(|hit| hit.title.as_str()),
        Some("Wayland-devel")
    );
    assert!(
        !found.hits[0].positions.is_empty(),
        "with what matched marked"
    );
    world.session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_folder_query_offers_every_folder_and_a_miss_says_so() {
    let world = world().await;

    let all = world.session.finder_folders(String::new()).await;
    assert!(all.hits.iter().any(|hit| hit.id == world.inbox));
    assert_eq!(all.hits.len(), 3);

    let none = world.session.finder_folders("zzz".to_owned()).await;
    assert!(none.hits.is_empty());
    assert_eq!(none.empty, "No folder matches “zzz”");
    world.session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_correspondent_is_found_and_becomes_a_from_query() {
    // Picking one writes `from:` into the box and drops back into search,
    // so what follows is an ordinary query -- GTK's behaviour.
    let world = world().await;

    let found = world.session.finder_contacts("ada".to_owned()).await;

    let hit = found.hits.first().expect("Ada");
    assert_eq!(hit.title, "Ada Lovelace");
    assert_eq!(hit.detail.as_deref(), Some("ada@example.com"));
    assert_eq!(hit.query.as_deref(), Some("from:ada@example.com"));
    world.session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_store_with_no_correspondents_says_where_they_come_from() {
    // Two empties: Postio has no address book, so none at all is a mailbox
    // that has not synced, not a query that missed.
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        test_support::account_with_inbox(&connection).await;
    }
    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");

    let found = session.finder_contacts("ada".to_owned()).await;

    assert!(found.hits.is_empty());
    assert!(
        found.empty.contains("learns them from the mail it syncs"),
        "{}",
        found.empty
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_picked_label_goes_on_the_message_under_the_cursor() {
    let world = world().await;
    let found = world.session.finder_labels("tax".to_owned()).await;
    assert_eq!(found.hits.first().map(|hit| hit.id), Some(world.label));

    world.session.invoke("first_message");
    world.session.apply_label(world.label);

    let labelled = settle_until(async || {
        let connection = world.database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .get(postio_model::ids::MessageId::new(world.message))
            .await
            .expect("a read")
            .is_some_and(|message| {
                message
                    .labels
                    .contains(&postio_model::ids::LabelId::new(world.label))
            })
    })
    .await;
    assert!(labelled, "the label never reached the message");
    world.session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_label_miss_says_so() {
    let world = world().await;
    let found = world.session.finder_labels("receipts".to_owned()).await;
    assert!(found.hits.is_empty());
    assert_eq!(found.empty, "No label matches “receipts”");
    world.session.shutdown();
}

async fn settle_until<F, Fut>(done: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while std::time::Instant::now() < deadline {
        if done().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    done().await
}
