//! Saving a sender from the new-sender popover (specs/011-focus-contacts S9):
//! what their signature said becomes theirs, only on Add contact, and
//! undoing it leaves nothing behind; ignoring a sender is a mark that
//! survives.

use postio_model::{ContactSettings, DetailKind, EmailAddress, SignatureDetails};
use postio_storage::repository::{ContactRepository, ContactSettingsRepository};
use postio_storage::test_support;

fn jonas() -> SignatureDetails {
    SignatureDetails {
        title: Some("Partnerships".into()),
        company: Some("Harbor Data Co.".into()),
        phones: vec!["+1 555 0142".into()],
        websites: vec!["harbordata.example".into()],
    }
}

#[tokio::test]
async fn add_contact_saves_the_person_and_keeps_what_the_signature_said() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let contacts = ContactRepository::new(&connection);
    let person = contacts
        .create(
            Some("Jonas Weber"),
            &[EmailAddress::new(
                None::<String>,
                "jonas@harbordata.example",
            )],
        )
        .await
        .unwrap();
    // Made by hand a person is saved already; make them a sender only.
    let prior = contacts.add_sender(person, &jonas()).await.unwrap();
    let after = contacts.get(person).await.unwrap().unwrap();
    assert!(after.is_saved());
    assert_eq!(after.title.as_deref(), Some("Partnerships"));
    assert_eq!(after.organization.as_deref(), Some("Harbor Data Co."));
    let kept = contacts.details(person).await.unwrap();
    assert_eq!(kept.len(), 2);
    assert!(kept.iter().all(|d| d.added));
    assert!(
        kept.iter()
            .any(|d| d.kind == DetailKind::Phone && d.value == "+1 555 0142")
    );

    contacts.restore_sender(person, &prior).await.unwrap();
    assert!(contacts.details(person).await.unwrap().is_empty());
    let back = contacts.get(person).await.unwrap().unwrap();
    assert_eq!(back.title, prior.title);
    assert_eq!(back.organization, prior.organization);
}

#[tokio::test]
async fn a_title_the_user_already_set_is_not_overwritten() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let contacts = ContactRepository::new(&connection);
    let person = contacts
        .create(
            Some("Jonas"),
            &[EmailAddress::new(
                None::<String>,
                "jonas@harbordata.example",
            )],
        )
        .await
        .unwrap();
    sqlx_title(&connection, person.get(), "Founder").await;
    contacts.add_sender(person, &jonas()).await.unwrap();
    let after = contacts.get(person).await.unwrap().unwrap();
    assert_eq!(after.title.as_deref(), Some("Founder"));
    assert_eq!(after.organization.as_deref(), Some("Harbor Data Co."));
}

async fn sqlx_title(connection: &postio_storage::Checkout, id: i64, title: &str) {
    postio_storage::sql::execute(
        connection,
        "UPDATE contacts SET title = ?2 WHERE id = ?1",
        postio_storage::sql::bind![id, title],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn an_ignored_detail_is_remembered_and_forgetting_it_offers_it_again() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let contacts = ContactRepository::new(&connection);
    let person = contacts
        .create(
            Some("Jonas"),
            &[EmailAddress::new(
                None::<String>,
                "jonas@harbordata.example",
            )],
        )
        .await
        .unwrap();
    contacts
        .set_detail(person, DetailKind::Phone, "+1 555 0142", Some(false))
        .await
        .unwrap();
    let seen = contacts.details(person).await.unwrap();
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].added, "ignored, not kept");
    contacts
        .set_detail(person, DetailKind::Phone, "+1 555 0142", None)
        .await
        .unwrap();
    assert!(contacts.details(person).await.unwrap().is_empty());
}

#[tokio::test]
async fn ignore_sender_is_a_setting_that_survives_being_the_only_one() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let contacts = ContactRepository::new(&connection);
    let person = contacts
        .create(
            Some("Jonas"),
            &[EmailAddress::new(
                None::<String>,
                "jonas@harbordata.example",
            )],
        )
        .await
        .unwrap();
    let settings = ContactSettingsRepository::new(&connection);
    let ignored = ContactSettings {
        ignored: true,
        ..ContactSettings::default()
    };
    assert!(!ignored.is_default(), "an ignored sender has a row");
    settings.put(person, &ignored).await.unwrap();
    assert!(settings.get(person).await.unwrap().ignored);
}
