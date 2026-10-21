//! Finding records that look like one person without being told which
//! (specs/011-focus-contacts S10): the body stage notes what a sender's mail
//! said (a signature, a phone, a `Resent-From`), and the review finds pairs
//! by it, and by one address stopping as another begins in the same threads.
//! A pair with only one kind of evidence is a candidate with that evidence.

use postio_model::{ContactId, EvidenceKind};
use postio_storage::repository::ContactRepository;

use super::contact_stats::{at, mail, person, setup};

async fn two_senders() -> (
    postio_storage::Store,
    postio_storage::Checkout,
    postio_model::MessageId,
    postio_model::MessageId,
) {
    let (database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    let old = mail(
        &connection,
        &account,
        inbox,
        at(2026, 1, 1),
        "a.moreno@example.net",
        &me,
        true,
    )
    .await;
    let new = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 1),
        "ada@example.org",
        &me,
        true,
    )
    .await;
    (database, connection, old, new)
}

async fn ids(connection: &postio_storage::Checkout) -> (ContactId, ContactId) {
    (
        person(connection, "a.moreno@example.net").await,
        person(connection, "ada@example.org").await,
    )
}

#[tokio::test]
async fn the_same_signature_alone_makes_a_candidate() {
    let (_db, connection, old, new) = two_senders().await;
    let contacts = ContactRepository::new(&connection);
    let key = "ada moreno|finance lead|northwind partners".to_owned();
    contacts
        .record_evidence(old, &[(EvidenceKind::Signature, key.clone())])
        .await
        .unwrap();
    assert!(
        contacts.discovered_pairs(10).await.unwrap().is_empty(),
        "one is not a pair"
    );
    contacts
        .record_evidence(new, &[(EvidenceKind::Signature, key)])
        .await
        .unwrap();

    let (a, b) = ids(&connection).await;
    let found = contacts.discovered_pairs(10).await.unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0] == (a.min(b), a.max(b)));
    let (x, y) = (
        contacts.get(a).await.unwrap().unwrap(),
        contacts.get(b).await.unwrap().unwrap(),
    );
    let evidence = contacts.pair_evidence(&x, &y).await.unwrap();
    assert_eq!(
        evidence.signature.as_deref(),
        Some("ada moreno|finance lead|northwind partners")
    );
    assert!(evidence.phone.is_none() && evidence.resent.is_none());
}

#[tokio::test]
async fn the_same_phone_alone_makes_a_candidate_and_says_which() {
    let (_db, connection, old, new) = two_senders().await;
    let contacts = ContactRepository::new(&connection);
    for message in [old, new] {
        contacts
            .record_evidence(message, &[(EvidenceKind::Phone, "+1 555 0119".to_owned())])
            .await
            .unwrap();
    }
    let (a, b) = ids(&connection).await;
    assert_eq!(
        contacts.discovered_pairs(10).await.unwrap(),
        [(a.min(b), a.max(b))]
    );
    let (x, y) = (
        contacts.get(a).await.unwrap().unwrap(),
        contacts.get(b).await.unwrap().unwrap(),
    );
    assert_eq!(
        contacts
            .pair_evidence(&x, &y)
            .await
            .unwrap()
            .phone
            .as_deref(),
        Some("+1 555 0119")
    );
}

#[tokio::test]
async fn a_resent_from_names_the_other_record_and_alone_makes_a_candidate() {
    let (_db, connection, _old, new) = two_senders().await;
    let contacts = ContactRepository::new(&connection);
    // The mail from ada@example.org carried `Resent-From: a.moreno@example.net`.
    contacts
        .record_evidence(
            new,
            &[(EvidenceKind::Resent, "a.moreno@example.net".to_owned())],
        )
        .await
        .unwrap();
    let (old_id, new_id) = ids(&connection).await;
    let found = contacts.discovered_pairs(10).await.unwrap();
    assert_eq!(found, [(old_id.min(new_id), old_id.max(new_id))]);
    let (x, y) = (
        contacts.get(old_id).await.unwrap().unwrap(),
        contacts.get(new_id).await.unwrap().unwrap(),
    );
    let evidence = contacts.pair_evidence(&x, &y).await.unwrap();
    assert_eq!(
        evidence.resent,
        Some((
            "a.moreno@example.net".to_owned(),
            "ada@example.org".to_owned()
        ))
    );
}

#[tokio::test]
async fn one_address_stopping_as_another_begins_in_the_same_threads_makes_a_candidate() {
    let (_db, connection, account, inbox) = {
        let (db, connection, account, inbox) = setup().await;
        (db, connection, account, inbox)
    };
    let me = account.address.address.clone();
    for n in 0..2u32 {
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
    let (a, b) = ids(&connection).await;
    assert_eq!(
        contacts.discovered_pairs(10).await.unwrap(),
        [(a.min(b), a.max(b))]
    );
}

#[tokio::test]
async fn a_pair_the_user_said_is_two_people_is_not_found_again() {
    let (_db, connection, old, new) = two_senders().await;
    let contacts = ContactRepository::new(&connection);
    for message in [old, new] {
        contacts
            .record_evidence(message, &[(EvidenceKind::Phone, "+1 555 0119".to_owned())])
            .await
            .unwrap();
    }
    let (a, b) = ids(&connection).await;
    contacts.dismiss(a, b).await.unwrap();
    assert!(contacts.discovered_pairs(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn evidence_is_kept_once_however_often_the_mail_says_it() {
    let (_db, connection, old, _new) = two_senders().await;
    let contacts = ContactRepository::new(&connection);
    for _ in 0..3 {
        contacts
            .record_evidence(old, &[(EvidenceKind::Phone, "+1 555 0119".to_owned())])
            .await
            .unwrap();
    }
    let count =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM contact_evidence", ())
            .await
            .unwrap();
    assert_eq!(count, 1);
}
