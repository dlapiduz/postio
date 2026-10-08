//! Recipient suggestions on the Mac (specs/009-focus-macos T074): the one
//! completion rule every app ranks by (`postio_ui::recipients::suggest`),
//! over the account's directory and, beside it, the people the Mac's own
//! Contacts lends for the keystroke.

use chrono::Utc;
use postio_ffi::{ExternalContactFfi, Session, SessionOptions};
use postio_model::EmailAddress;
use postio_storage::repository::{ContactGroupRepository, ContactRepository};
use postio_storage::test_support;

/// A session over an account whose directory holds a correspondent written
/// to three times, a contact the person deleted, and a group; and the
/// account's id.
async fn directory() -> (std::sync::Arc<Session>, i64) {
    let database = test_support::memory().await;
    let account = {
        let connection = database.connect().await.expect("a connection");
        let (account, _inbox) = test_support::account_with_inbox(&connection).await;
        let contacts = ContactRepository::new(&connection);
        let at = Utc::now();
        let wrote = EmailAddress::new(Some("Adams Field"), "adams@example.org");
        let member = contacts
            .record(Some(account.id), &wrote, at)
            .await
            .expect("a sighting");
        let gone = contacts
            .record(
                Some(account.id),
                &EmailAddress::new(None::<String>, "adamant@example.net"),
                at,
            )
            .await
            .expect("a sighting");
        contacts.delete(gone).await.expect("suppressed");
        for statement in [
            "INSERT INTO addresses (address, address_normalized)
             VALUES ('adams@example.org', 'adams@example.org')",
            "INSERT INTO correspondents (address_id, sent_count, last_sent_at)
             SELECT id, 3, NULL FROM addresses
              WHERE address_normalized = 'adams@example.org'",
        ] {
            postio_storage::sql::execute(&connection, statement, ())
                .await
                .expect("written to three times");
        }
        let groups = ContactGroupRepository::new(&connection);
        let mut group =
            postio_model::contact_group::ContactGroup::new(Some(account.id), "Adam crew", at);
        let group = groups.create(&mut group).await.expect("a group");
        groups.add_member(group, member).await.expect("a member");
        account.id
    };
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    (session, account.get())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_then_who_was_written_to_then_the_address_book() {
    let (session, account) = directory().await;
    let extra = vec![ExternalContactFfi {
        name: Some("Adamina Example".into()),
        address: "adamina@example.com".into(),
    }];
    let offered: Vec<String> = session
        .recipient_suggestions(account, "adam".into(), 8, extra)
        .into_iter()
        .map(|suggestion| suggestion.label)
        .collect();
    assert_eq!(
        offered,
        [
            "Adam crew (1 people)",
            "Adams Field <adams@example.org>",
            "Adamina Example <adamina@example.com>",
        ],
        "the group by name, the correspondent written to, then the Mac's \
         contact; the deleted contact never"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn accepting_a_suggestion_completes_the_field() {
    let (session, account) = directory().await;
    let suggestions =
        session.recipient_suggestions(account, "bea@example.com, adams".into(), 8, Vec::new());
    let adams = suggestions
        .iter()
        .find(|suggestion| suggestion.label.contains("adams@example.org"))
        .expect("offered");
    assert!(
        adams.accepted.starts_with("bea@example.com, ")
            && adams.accepted.contains("adams@example.org"),
        "the entry being typed is replaced, the rest kept: {}",
        adams.accepted
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn too_short_a_prefix_offers_nothing() {
    let (session, account) = directory().await;
    assert!(
        session
            .recipient_suggestions(account, "ada".into(), 8, Vec::new())
            .is_empty(),
        "three letters match most of an address book (#424)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_address_book_entry_already_in_the_directory_is_offered_once() {
    let (session, account) = directory().await;
    let extra = vec![ExternalContactFfi {
        name: Some("A. Field".into()),
        address: "ADAMS@example.org".into(),
    }];
    let offered = session.recipient_suggestions(account, "adams".into(), 8, extra);
    assert_eq!(offered.len(), 1, "{offered:?}");
}
