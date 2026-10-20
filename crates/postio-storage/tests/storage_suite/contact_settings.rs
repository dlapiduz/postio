//! What the user decided about a correspondent, kept on the correspondent
//! (specs/011-focus-contacts S3, ADR 0048).

use postio_model::{
    ContactArrival, ContactId, ContactSettings, DigestSetting, EmailAddress, StoppedMarker,
};
use postio_storage::repository::{ContactRepository, ContactSettingsRepository};
use postio_storage::test_support::counting::{counted_async, unbounded};
use postio_storage::{Checkout, test_support};

async fn person(connection: &Checkout, email: &str) -> ContactId {
    ContactRepository::new(connection)
        .person_for_email(&EmailAddress::new(None::<String>, email))
        .await
        .expect("a person")
}

fn digest(name: &str) -> DigestSetting {
    DigestSetting {
        name: name.into(),
        cadence: "weekly".into(),
        day: Some("saturday".into()),
        at: "09:00".into(),
    }
}

#[tokio::test]
async fn a_person_is_made_for_an_address_the_store_has_never_seen_and_only_once() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let ada = person(&connection, "Ada@Example.org").await;
    assert_eq!(person(&connection, "ada@example.org").await, ada);
    let stored = ContactRepository::new(&connection)
        .get(ada)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !stored.is_saved(),
        "being given a setting is not being saved"
    );
    assert_eq!(stored.addresses.len(), 1);
}

#[tokio::test]
async fn settings_round_trip_and_a_default_row_is_no_row() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let ada = person(&connection, "ada@example.org").await;
    let settings = ContactSettingsRepository::new(&connection);
    assert!(settings.get(ada).await.unwrap().is_default());

    let mut wanted = ContactSettings {
        arrival: Some(ContactArrival::Digest),
        digest: Some(digest("Ledger")),
        html_treatment: Some("paper".into()),
        remind_days: Some(3),
        ..ContactSettings::default()
    };
    wanted.set_stop(StoppedMarker::Question, true);
    settings.put(ada, &wanted).await.unwrap();
    assert_eq!(settings.get(ada).await.unwrap(), wanted);

    settings
        .put(ada, &ContactSettings::default())
        .await
        .unwrap();
    let rows: i64 =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM contact_settings", ())
            .await
            .unwrap();
    assert_eq!(rows, 0, "nothing decided keeps no row");
}

#[tokio::test]
async fn the_engine_reads_every_live_persons_decisions_with_their_addresses_in_one_statement() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let contacts = ContactRepository::new(&connection);
    let settings = ContactSettingsRepository::new(&connection);
    let ada = contacts
        .create(
            Some("Ada Lovelace"),
            &[
                EmailAddress::new(None::<String>, "ada@work.example"),
                EmailAddress::new(None::<String>, "ada@home.example"),
            ],
        )
        .await
        .unwrap();
    let bob = person(&connection, "bob@example.org").await;
    let carol = person(&connection, "carol@example.org").await;
    let pinned = ContactSettings {
        arrival: Some(ContactArrival::Inbox),
        ..ContactSettings::default()
    };
    settings.put(ada, &pinned).await.unwrap();
    settings
        .put(
            bob,
            &ContactSettings {
                html_treatment: Some("app".into()),
                ..ContactSettings::default()
            },
        )
        .await
        .unwrap();
    settings.put(carol, &pinned).await.unwrap();
    contacts.delete(carol).await.unwrap();

    let mut derived = Vec::new();
    let cost = counted_async(|| async {
        derived = settings.derived().await.unwrap();
    })
    .await;

    assert_eq!(cost.statements, 1);
    assert_eq!(
        derived.len(),
        1,
        "a treatment is not a rule, and a deleted person's rules rest"
    );
    let ada = &derived[0];
    assert_eq!(ada.name, "Ada Lovelace");
    let mut addresses = ada.addresses.clone();
    addresses.sort();
    assert_eq!(
        addresses,
        ["ada@home.example", "ada@work.example"],
        "all of the person's addresses"
    );
    assert_eq!(ada.settings.arrival, Some(ContactArrival::Inbox));
    assert_eq!(
        unbounded(
            &connection,
            ContactSettingsRepository::explain_derived(),
            &["contacts", "addresses"]
        )
        .await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn a_treatment_is_found_by_any_address_of_the_person() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let contacts = ContactRepository::new(&connection);
    let settings = ContactSettingsRepository::new(&connection);
    let ada = contacts
        .create(
            Some("Ada"),
            &[
                EmailAddress::new(None::<String>, "ada@work.example"),
                EmailAddress::new(None::<String>, "ada@home.example"),
            ],
        )
        .await
        .unwrap();
    settings
        .put(
            ada,
            &ContactSettings {
                html_treatment: Some("paper".into()),
                ..ContactSettings::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        settings
            .treatment_for("Ada@Home.example")
            .await
            .unwrap()
            .as_deref(),
        Some("paper")
    );
    assert_eq!(
        settings.treatment_for("nobody@example.org").await.unwrap(),
        None
    );
    assert_eq!(
        settings.treatments().await.unwrap().len(),
        2,
        "one per address, for a reader that looks up by sender"
    );
}

#[tokio::test]
async fn a_merge_carries_the_absorbed_persons_decisions_and_an_unjoin_takes_them_back() {
    let database = test_support::memory().await;
    let connection = database.connect().await.unwrap();
    let contacts = ContactRepository::new(&connection);
    let settings = ContactSettingsRepository::new(&connection);
    let kept = person(&connection, "ada@work.example").await;
    let absorbed = person(&connection, "ada@home.example").await;
    let mut mine = ContactSettings {
        html_treatment: Some("app".into()),
        ..ContactSettings::default()
    };
    mine.set_stop(StoppedMarker::Todo, true);
    settings.put(kept, &mine).await.unwrap();
    let mut theirs = ContactSettings {
        arrival: Some(ContactArrival::Digest),
        digest: Some(digest("Ada's letters")),
        html_treatment: Some("paper".into()),
        ..ContactSettings::default()
    };
    theirs.set_stop(StoppedMarker::Question, true);
    settings.put(absorbed, &theirs).await.unwrap();

    let receipt = contacts
        .join(kept, &[absorbed], "Ada Lovelace", None)
        .await
        .unwrap();

    let after = settings.get(kept).await.unwrap();
    assert_eq!(
        after.arrival,
        Some(ContactArrival::Digest),
        "a gap: the absorbed's digest"
    );
    assert_eq!(after.digest, Some(digest("Ada's letters")));
    assert_eq!(
        after.html_treatment.as_deref(),
        Some("app"),
        "both held one: the kept's"
    );
    assert_eq!(
        after.stopped(),
        [StoppedMarker::Question, StoppedMarker::Todo]
    );
    // The engine reads her once, covering both addresses.
    let derived = settings.derived().await.unwrap();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].addresses.len(), 2);

    contacts.unjoin(&receipt).await.unwrap();
    assert_eq!(
        settings.get(kept).await.unwrap(),
        mine,
        "the kept person as they were"
    );
    assert_eq!(
        settings.get(absorbed).await.unwrap(),
        theirs,
        "and the absorbed"
    );
}
