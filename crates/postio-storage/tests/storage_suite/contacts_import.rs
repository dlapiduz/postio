//! A vCard import applied to the store (specs/005-contacts User Story 6,
//! FR-053, R9): a card whose address the mail already knows is that person,
//! a card spanning several people joins them and says so, the user's own
//! name for someone beats the card's, and the card itself is kept.

use chrono::{TimeZone, Utc};

use postio_model::card::{Member, ParsedCard, ParsedGroup, ParsedPerson};
use postio_model::{Account, ContactSource, EmailAddress, MailboxId, Message};
use postio_storage::Connection;
use postio_storage::repository::{ContactGroupRepository, ContactRepository};
use postio_storage::test_support;

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
    email: &str,
    times: u32,
) {
    for day in 0..times {
        let mut message = Message::new(
            account.id,
            inbox,
            Utc.with_ymd_and_hms(2026, 3, 1 + day, 12, 0, 0).unwrap(),
        );
        message.from = vec![EmailAddress::new(Some("from mail"), email)];
        ContactRepository::new(connection)
            .record_message(&message, std::slice::from_ref(&account.address))
            .await
            .expect("record");
    }
}

fn card(uid: &str, name: &str, emails: &[&str]) -> ParsedCard {
    ParsedCard::Person(ParsedPerson {
        uid: Some(uid.into()),
        name: Some(name.into()),
        emails: emails
            .iter()
            .enumerate()
            .map(|(i, e)| (EmailAddress::new(None::<String>, *e), i == 0))
            .collect(),
        organization: Some("Analytical Engines".into()),
        note: None,
        raw: format!(
            "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{name}\r\nX-KEPT:yes\r\nEND:VCARD\r\n"
        ),
    })
}

#[tokio::test]
async fn a_card_for_an_address_the_mail_knows_is_that_person() {
    let (_db, connection, account, inbox) = setup().await;
    from(&connection, &account, inbox, "ada@work.example", 3).await;
    let contacts = ContactRepository::new(&connection);
    let known = contacts
        .by_address("ada@work.example")
        .await
        .expect("lookup")
        .expect("ada");

    let summary = contacts
        .apply_import(&[card("urn:uuid:a", "Ada Lovelace", &["ada@work.example"])])
        .await
        .expect("import");

    assert_eq!((summary.people_created, summary.people_updated), (0, 1));
    let ada = contacts
        .get(known.id)
        .await
        .expect("get")
        .expect("same person");
    assert_eq!(ada.name.as_deref(), Some("Ada Lovelace"), "the card's name");
    assert_eq!(ada.organization.as_deref(), Some("Analytical Engines"));
    assert_eq!(ada.times_seen, 3, "and the address's history");
    assert_eq!(ada.source, ContactSource::Import);
    let (uid, vcard) = contacts.card_of(known.id).await.expect("card");
    assert_eq!(uid.as_deref(), Some("urn:uuid:a"));
    assert!(
        vcard.expect("stored").contains("X-KEPT:yes"),
        "the card itself is kept"
    );
}

#[tokio::test]
async fn a_card_spanning_two_people_joins_them_and_says_so() {
    let (_db, connection, account, inbox) = setup().await;
    from(&connection, &account, inbox, "ada@work.example", 2).await;
    from(&connection, &account, inbox, "ada@home.example", 1).await;
    let contacts = ContactRepository::new(&connection);
    let summary = contacts
        .apply_import(&[card(
            "urn:uuid:a",
            "Ada Lovelace",
            &["ada@work.example", "ada@home.example", "ada@new.example"],
        )])
        .await
        .expect("import");
    assert_eq!(summary.joins.len(), 1, "{summary:?}");
    let work = contacts
        .by_address("ada@work.example")
        .await
        .expect("lookup")
        .expect("ada");
    let home = contacts
        .by_address("ada@home.example")
        .await
        .expect("lookup")
        .expect("ada");
    let new = contacts
        .by_address("ada@new.example")
        .await
        .expect("lookup")
        .expect("ada");
    assert_eq!(
        (work.id, home.id),
        (new.id, new.id),
        "one person, every address"
    );
    assert_eq!(work.times_seen, 3);
}

#[tokio::test]
async fn the_users_name_beats_the_cards_and_is_counted() {
    let (_db, connection, _account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .create(
            Some("Ada, my friend"),
            &[EmailAddress::new(None::<String>, "ada@work.example")],
        )
        .await
        .expect("ada");
    let summary = contacts
        .apply_import(&[card("urn:uuid:a", "Ada Lovelace", &["ada@work.example"])])
        .await
        .expect("import");
    assert_eq!(summary.name_conflicts, 1);
    assert_eq!(
        contacts
            .get(ada)
            .await
            .expect("get")
            .expect("ada")
            .name
            .as_deref(),
        Some("Ada, my friend")
    );
}

#[tokio::test]
async fn a_new_person_and_a_group_of_them_by_uid_and_address() {
    let (_db, connection, _account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let summary = contacts
        .apply_import(&[
            ParsedCard::Group(ParsedGroup {
                uid: Some("urn:uuid:g".into()),
                name: "Family".into(),
                members: vec![
                    Member::Uid("urn:uuid:a".into()),
                    Member::Address("grace@example.org".into()),
                ],
                raw: "BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:Family\r\nEND:VCARD\r\n"
                    .into(),
            }),
            card("urn:uuid:a", "Ada Lovelace", &["ada@work.example"]),
            card("urn:uuid:g2", "Grace Hopper", &["grace@example.org"]),
        ])
        .await
        .expect("import");
    assert_eq!((summary.people_created, summary.groups_created), (2, 1));
    let groups = ContactGroupRepository::new(&connection);
    let family = groups.list().await.expect("list").remove(0);
    let mut names: Vec<String> = groups
        .members(family.id)
        .await
        .expect("members")
        .iter()
        .map(|p| p.display_name().to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["Ada Lovelace", "Grace Hopper"],
        "people first, then their groups"
    );
}

#[tokio::test]
async fn export_names_everyone_the_same_way_twice_and_never_a_deleted_person() {
    let (_db, connection, _account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .create(
            Some("Ada"),
            &[EmailAddress::new(None::<String>, "ada@work.example")],
        )
        .await
        .expect("ada");
    let gone = contacts
        .create(
            Some("Gone"),
            &[EmailAddress::new(None::<String>, "gone@example.org")],
        )
        .await
        .expect("gone");
    contacts.delete(gone).await.expect("delete");

    let ids = contacts
        .view_ids(postio_model::ContactView::Written)
        .await
        .expect("ids");
    assert_eq!(ids, [ada], "the list's own people, and nobody deleted");
    let first = contacts.export_people(&[ada, gone]).await.expect("export");
    assert_eq!(first.len(), 1, "a deleted person is never exported");
    assert!(first[0].uid.starts_with("urn:uuid:"));
    let again = contacts.export_people(&[ada]).await.expect("export");
    assert_eq!(again[0].uid, first[0].uid, "the UID given is kept");
}
