//! A person the user makes, edits, deletes and restores
//! (specs/005-contacts User Story 3, FR-020..FR-025, SC-005).
//!
//! Every step is what the next one undoes: `edit` hands back the fields it
//! replaced, `delete` the state it left, and putting those back is exact.

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{
    Account, ContactSource, ContactState, ContactView, EmailAddress, MailboxId, Message, PersonEdit,
};
use postio_storage::Connection;
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;

fn at(days: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

fn address(email: &str) -> EmailAddress {
    EmailAddress::new(None::<String>, email)
}

async fn setup() -> (
    postio_storage::Store,
    postio_storage::Checkout,
    Account,
    MailboxId,
) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (database, connection, account, inbox)
}

async fn from(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
    name: &str,
    email: &str,
    day: i64,
) {
    let mut message = Message::new(account.id, inbox, at(day));
    message.from = vec![EmailAddress::new(Some(name), email)];
    ContactRepository::new(connection)
        .record_message(&message, std::slice::from_ref(&account.address))
        .await
        .expect("record");
}

async fn completes(connection: &Connection, prefix: &str) -> Vec<String> {
    ContactRepository::new(connection)
        .complete(prefix, 8)
        .await
        .expect("complete")
        .into_iter()
        .map(|p| p.display_name().to_owned())
        .collect()
}

#[tokio::test]
async fn a_made_person_is_offered_at_once() {
    let (_db, connection, _account, _inbox) = setup().await;
    let made = ContactRepository::new(&connection)
        .create(Some("Grace Hopper"), &[address("grace@example.org")])
        .await
        .expect("create");
    let grace = ContactRepository::new(&connection)
        .get(made)
        .await
        .expect("get")
        .expect("grace");
    assert_eq!(grace.source, ContactSource::User);
    assert_eq!(completes(&connection, "gra").await, ["Grace Hopper"]);
}

#[tokio::test]
async fn the_users_own_address_cannot_be_a_contact() {
    let (_db, connection, account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let refused = contacts
        .create(Some("Me"), std::slice::from_ref(&account.address))
        .await;
    assert!(
        matches!(&refused, Err(postio_storage::Error::ForbiddenTransition { reason, .. }) if reason.contains("yours")),
        "the reason says why: {refused:?}"
    );
    let ada = contacts
        .create(Some("Ada"), &[address("ada@example.com")])
        .await
        .expect("ada");
    assert!(
        contacts.add_address(ada, &account.address).await.is_err(),
        "nor can it be added to someone"
    );
}

#[tokio::test]
async fn an_edit_promotes_in_place_and_mail_never_renames_it() {
    let (_db, connection, account, inbox) = setup().await;
    from(&connection, &account, inbox, "A. L.", "ada@example.com", 0).await;
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .by_address("ada@example.com")
        .await
        .expect("lookup")
        .expect("ada");
    assert_eq!(ada.source, ContactSource::Mail);

    let prior = contacts
        .edit(
            ada.id,
            &PersonEdit {
                name: Some("Ada Lovelace".into()),
                organization: Some("Analytical Engines".into()),
                note: Some("met at the conference".into()),
            },
        )
        .await
        .expect("edit");
    let edited = contacts.get(ada.id).await.expect("get").expect("ada");
    assert_eq!(edited.id, ada.id, "in place");
    assert_eq!(edited.source, ContactSource::User, "promoted (FR-022)");
    assert_eq!(edited.name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(
        completes(&connection, "analyt").await,
        ["Ada Lovelace"],
        "the filter index follows"
    );

    from(
        &connection,
        &account,
        inbox,
        "Someone Else",
        "ada@example.com",
        1,
    )
    .await;
    assert_eq!(
        contacts
            .get(ada.id)
            .await
            .expect("get")
            .expect("ada")
            .name
            .as_deref(),
        Some("Ada Lovelace"),
        "a header never renames a person the user named (FR-021)"
    );

    // What the edit replaced puts it back exactly.
    contacts.put_fields(ada.id, &prior).await.expect("put back");
    let back = contacts.get(ada.id).await.expect("get").expect("ada");
    assert_eq!(back.source, ContactSource::Mail);
    assert_eq!(back.name, None);
    assert_eq!(back.organization, None);
}

#[tokio::test]
async fn a_deleted_person_is_nowhere_but_the_deleted_view_and_stays_deleted() {
    let (_db, connection, account, inbox) = setup().await;
    from(
        &connection,
        &account,
        inbox,
        "Robot",
        "robot@example.com",
        0,
    )
    .await;
    let contacts = ContactRepository::new(&connection);
    let robot = contacts
        .by_address("robot@example.com")
        .await
        .expect("lookup")
        .expect("robot");

    let was = contacts.delete(robot.id).await.expect("delete");
    assert_eq!(was, ContactState::Live);
    assert!(
        completes(&connection, "rob").await.is_empty(),
        "not offered"
    );
    assert_eq!(
        contacts.count(ContactView::Everyone).await.expect("count"),
        0
    );
    assert_eq!(
        contacts.count(ContactView::Deleted).await.expect("count"),
        1
    );

    // More mail from it counts on it, and it stays deleted (SC-005).
    from(
        &connection,
        &account,
        inbox,
        "Robot",
        "robot@example.com",
        1,
    )
    .await;
    let still = contacts.get(robot.id).await.expect("get").expect("robot");
    assert_eq!(still.state, ContactState::Deleted);
    assert_eq!(still.times_seen, 2);
    assert!(completes(&connection, "rob").await.is_empty());

    // Restored whole, with what arrived meanwhile.
    contacts.restore(robot.id, was).await.expect("restore");
    let back = contacts.get(robot.id).await.expect("get").expect("robot");
    assert_eq!(back.state, ContactState::Live);
    assert_eq!(back.times_seen, 2);
    assert_eq!(completes(&connection, "rob").await, ["Robot"]);
}
