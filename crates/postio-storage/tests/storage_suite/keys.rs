use postio_model::EmailAddress;
use postio_storage::repository::{KeyKind, KeyRepository, KeySource, NewKey, VerdictRow};
use postio_storage::test_support;

fn ada() -> EmailAddress {
    EmailAddress::new(None::<String>, "ada@example.com")
}

fn key<'a>(
    fingerprint: &'a str,
    source: KeySource,
    addresses: &'a [EmailAddress],
    at: i64,
) -> NewKey<'a> {
    NewKey {
        fingerprint,
        kind: KeyKind::OpenPgp,
        keyblock: b"keyblock bytes",
        primary_uid: Some("Ada Moreno <ada@example.com>"),
        revoked: false,
        addresses,
        source,
        message: None,
        at,
    }
}

#[tokio::test]
async fn a_key_is_kept_with_every_source_and_the_addresses_its_user_ids_name() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let keys = KeyRepository::new(&connection);
    let addresses = [ada()];
    keys.record(&key("AAAA", KeySource::Autocrypt, &addresses, 100))
        .await
        .unwrap();
    keys.record(&key("AAAA", KeySource::Gnupg, &addresses, 200))
        .await
        .unwrap();
    keys.record(&key("BBBB", KeySource::File, &addresses, 150))
        .await
        .unwrap();

    let held = keys.for_address(&ada()).await.unwrap();
    let seen: Vec<(&str, i64, i64)> = held
        .iter()
        .map(|key| (key.fingerprint.as_str(), key.first_seen, key.last_seen))
        .collect();
    assert_eq!(seen, [("AAAA", 100, 200), ("BBBB", 150, 150)]);
    assert_eq!(
        keys.sources("AAAA").await.unwrap(),
        [KeySource::Autocrypt, KeySource::Gnupg]
    );
    assert!(
        keys.for_address(&EmailAddress::new(None::<String>, "grace@example.net"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn acceptance_is_one_key_per_address_on_the_contact_and_comes_back_off() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let keys = KeyRepository::new(&connection);
    let addresses = [ada()];
    keys.record(&key("AAAA", KeySource::Autocrypt, &addresses, 100))
        .await
        .unwrap();
    assert_eq!(keys.accepted(&ada()).await.unwrap(), None);
    keys.accept(&ada(), "AAAA", 300).await.unwrap();
    assert_eq!(
        keys.accepted(&ada()).await.unwrap(),
        Some(("AAAA".to_owned(), 300))
    );
    keys.accept(&ada(), "BBBB", 400).await.unwrap();
    assert_eq!(
        keys.accepted(&ada()).await.unwrap().map(|(key, _)| key),
        Some("BBBB".to_owned()),
        "one accepted key per address: the newer replaces it"
    );
    keys.unaccept(&ada()).await.unwrap();
    assert_eq!(keys.accepted(&ada()).await.unwrap(), None);
}

#[tokio::test]
async fn switching_a_source_off_forgets_keys_held_by_it_alone_unless_accepted() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let keys = KeyRepository::new(&connection);
    let addresses = [ada()];
    keys.record(&key("ONLY", KeySource::Gnupg, &addresses, 1))
        .await
        .unwrap();
    keys.record(&key("BOTH", KeySource::Gnupg, &addresses, 1))
        .await
        .unwrap();
    keys.record(&key("BOTH", KeySource::Autocrypt, &addresses, 2))
        .await
        .unwrap();
    keys.record(&key("KEPT", KeySource::Gnupg, &addresses, 1))
        .await
        .unwrap();
    keys.accept(&ada(), "KEPT", 3).await.unwrap();

    keys.drop_source(KeySource::Gnupg).await.unwrap();
    let left: Vec<String> = keys
        .for_address(&ada())
        .await
        .unwrap()
        .into_iter()
        .map(|key| key.fingerprint)
        .collect();
    assert_eq!(left, ["BOTH", "KEPT"]);
    assert_eq!(keys.sources("BOTH").await.unwrap(), [KeySource::Autocrypt]);
}

#[tokio::test]
async fn a_verdict_is_one_row_per_message_read_back_by_its_id() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    let message = postio_storage::repository::MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .unwrap();
    let keys = KeyRepository::new(&connection);
    assert_eq!(keys.verdict(message).await.unwrap(), None);
    let row = VerdictRow {
        format: "pgp_mime".to_owned(),
        scope: "whole".to_owned(),
        status: "good".to_owned(),
        reason: None,
        fingerprint: Some("AAAA".to_owned()),
        checked_at: 500,
    };
    keys.put_verdict(message, &row).await.unwrap();
    assert_eq!(keys.verdict(message).await.unwrap(), Some(row.clone()));
    let bad = VerdictRow {
        status: "bad".to_owned(),
        reason: Some("tampered".to_owned()),
        ..row
    };
    keys.put_verdict(message, &bad).await.unwrap();
    assert_eq!(
        keys.verdict(message).await.unwrap(),
        Some(bad),
        "a re-check replaces it"
    );
}

#[tokio::test]
async fn a_contacts_card_lists_their_keys_with_sources_and_acceptance() {
    use postio_model::CardKey;
    use postio_storage::repository::ContactRepository;
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let contacts = ContactRepository::new(&connection);
    let person = contacts.person_for_email(&ada()).await.unwrap();
    let keys = KeyRepository::new(&connection);
    let addresses = [ada()];
    keys.record(&key("AAAA", KeySource::Autocrypt, &addresses, 100))
        .await
        .unwrap();
    keys.record(&key("AAAA", KeySource::File, &addresses, 200))
        .await
        .unwrap();
    keys.record(&key("BBBB", KeySource::Gnupg, &addresses, 300))
        .await
        .unwrap();
    keys.record(&key(
        "CCCC",
        KeySource::File,
        &[EmailAddress::new(None::<String>, "grace@example.net")],
        50,
    ))
    .await
    .unwrap();
    keys.accept(&ada(), "AAAA", 400).await.unwrap();

    let card = contacts.card(person, 0).await.unwrap().expect("a card");
    assert_eq!(
        card.keys,
        [
            CardKey {
                fingerprint: "AAAA".to_owned(),
                sources: vec!["autocrypt".to_owned(), "file".to_owned()],
                address: "ada@example.com".to_owned(),
                first_seen: 100,
                revoked: false,
                accepted: true,
            },
            CardKey {
                fingerprint: "BBBB".to_owned(),
                sources: vec!["gnupg".to_owned()],
                address: "ada@example.com".to_owned(),
                first_seen: 300,
                revoked: false,
                accepted: false,
            },
        ],
        "grace's key is not ada's"
    );
}
