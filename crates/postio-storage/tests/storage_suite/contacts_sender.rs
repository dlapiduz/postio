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

#[tokio::test]
async fn one_address_that_stopped_as_another_began_in_the_same_threads_is_found() {
    use super::contact_stats::{at, mail, person, setup};
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    for n in 0..3u32 {
        let old = mail(
            &connection,
            &account,
            inbox,
            at(2026, 1, 1 + n),
            "a.moreno@example.net",
            &me,
            true,
        )
        .await;
        let new = mail(
            &connection,
            &account,
            inbox,
            at(2026, 2, 1 + n),
            "ada@example.org",
            &me,
            true,
        )
        .await;
        postio_storage::sql::execute(
            &connection,
            "INSERT INTO threads (account_id, subject) VALUES (?1, ?2)",
            postio_storage::sql::bind![account.id.get(), format!("t{n}")],
        )
        .await
        .unwrap();
        let thread = connection.last_insert_rowid();
        for id in [old, new] {
            postio_storage::sql::execute(
                &connection,
                "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
                postio_storage::sql::bind![id.get(), thread],
            )
            .await
            .unwrap();
        }
    }
    let contacts = ContactRepository::new(&connection);
    let old = contacts
        .get(person(&connection, "a.moreno@example.net").await)
        .await
        .unwrap()
        .unwrap();
    let new = contacts
        .get(person(&connection, "ada@example.org").await)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        contacts.took_over(&new, &old).await.unwrap(),
        Some((
            "a.moreno@example.net".to_owned(),
            "ada@example.org".to_owned(),
            3
        )),
        "whichever order they are asked in"
    );
    let stranger = mail(
        &connection,
        &account,
        inbox,
        at(2026, 3, 1),
        "ben@example.net",
        &me,
        true,
    )
    .await;
    let _ = stranger;
    let ben = contacts
        .get(person(&connection, "ben@example.net").await)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(contacts.took_over(&ben, &new).await.unwrap(), None);
}

#[tokio::test]
async fn a_bounce_marks_the_address_counts_in_the_review_and_shows_on_the_card() {
    use super::contact_stats::{at, mail, person, setup};
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    mail(
        &connection,
        &account,
        inbox,
        at(2026, 1, 1),
        "a.moreno@example.net",
        &me,
        true,
    )
    .await;
    let contacts = ContactRepository::new(&connection);
    assert_eq!(contacts.overview().await.unwrap().review.bounced, 0);

    let when = chrono::Utc::now() - chrono::Duration::days(2);
    assert!(
        contacts
            .mark_bounced("A.Moreno@Example.NET", when)
            .await
            .unwrap()
    );
    assert!(
        !contacts
            .mark_bounced("nobody@example.net", when)
            .await
            .unwrap(),
        "an address the store never saw is not marked"
    );
    assert_eq!(contacts.overview().await.unwrap().review.bounced, 1);

    let ada = person(&connection, "a.moreno@example.net").await;
    let notes = contacts.address_notes(ada).await.unwrap();
    assert!(notes[0].bounced && notes[0].bounced_at.is_some());

    // A report older than one already known changes nothing.
    contacts
        .mark_bounced("a.moreno@example.net", when - chrono::Duration::days(9))
        .await
        .unwrap();
    let again = contacts.address_notes(ada).await.unwrap();
    assert_eq!(again[0].bounced_at, notes[0].bounced_at);

    // Long ago is not "recently".
    contacts
        .mark_bounced(
            "a.moreno@example.net",
            chrono::Utc::now() - chrono::Duration::days(200),
        )
        .await
        .unwrap();
    assert_eq!(contacts.overview().await.unwrap().review.bounced, 1);
}
