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
        title: None,
        phones: Vec::new(),
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
async fn a_card_spanning_two_people_offers_the_merge_and_changes_nobody() {
    // Specs/011-focus-contacts S11: an import never merges people behind the
    // user's back. A card that lists two people's addresses is the evidence
    // the review shows; the people stay as they are until they say so.
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
    assert_eq!(summary.offered_merges, 1, "{summary:?}");
    let work = contacts
        .by_address("ada@work.example")
        .await
        .unwrap()
        .unwrap();
    let home = contacts
        .by_address("ada@home.example")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(work.id, home.id, "still two people");
    let found = contacts.discovered_pairs(10).await.unwrap();
    assert_eq!(found, [(work.id.min(home.id), work.id.max(home.id))]);
    let evidence = contacts.pair_evidence(&work, &home).await.unwrap();
    assert!(evidence.card.is_some(), "the card is the evidence");
}

#[tokio::test]
async fn a_card_with_the_name_of_someone_you_have_and_another_address_is_offered_not_duplicated_silently()
 {
    let (_db, connection, account, inbox) = setup().await;
    from(&connection, &account, inbox, "ada@work.example", 2).await;
    let contacts = ContactRepository::new(&connection);
    contacts
        .apply_import(&[card("urn:uuid:a", "Ada Lovelace", &["ada@work.example"])])
        .await
        .expect("a first import names her");
    let summary = contacts
        .apply_import(&[card(
            "urn:uuid:b",
            "Ada Lovelace",
            &["ada@elsewhere.example"],
        )])
        .await
        .expect("a second card, another address");
    assert_eq!(summary.people_created, 1);
    assert_eq!(
        summary.look_alikes, 1,
        "she looks like someone you have: {summary:?}"
    );
    let made = contacts
        .by_address("ada@elsewhere.example")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        summary.made,
        [made.id],
        "who the import made, for the cursor"
    );
    assert!(
        summary.touched.contains(&made.id),
        "and who it touched, for the review"
    );
    let suggestions = contacts.suggestion_pairs(10).await.unwrap();
    assert_eq!(suggestions.len(), 1, "and the review offers the merge");
}

#[tokio::test]
async fn a_cards_phone_is_noted_so_a_second_record_with_it_is_found() {
    let (_db, connection, account, inbox) = setup().await;
    from(&connection, &account, inbox, "ada@work.example", 1).await;
    let contacts = ContactRepository::new(&connection);
    let mut first = card("urn:uuid:a", "Ada L", &["ada@work.example"]);
    let mut second = card("urn:uuid:b", "A. Lovelace", &["al@elsewhere.example"]);
    for (card, phone) in [(&mut first, "+1 555 0119"), (&mut second, "+1 555 0119")] {
        if let ParsedCard::Person(person) = card {
            person.phones = vec![phone.to_owned()];
        }
    }
    contacts
        .apply_import(&[first, second])
        .await
        .expect("import");
    let found = contacts.discovered_pairs(10).await.unwrap();
    assert_eq!(found.len(), 1, "one pair, found by the phone they share");
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

#[tokio::test]
async fn a_group_goes_out_with_the_members_that_do() {
    let (_db, connection, _account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let groups = ContactGroupRepository::new(&connection);
    let ada = contacts
        .create(
            Some("Ada"),
            &[EmailAddress::new(None::<String>, "ada@work.example")],
        )
        .await
        .expect("ada");
    let grace = contacts
        .create(
            Some("Grace"),
            &[EmailAddress::new(None::<String>, "grace@example.org")],
        )
        .await
        .expect("grace");
    let mut family = postio_model::ContactGroup::new("Family", Utc::now());
    let family = groups.create(&mut family).await.expect("family");
    groups
        .add_members(family, &[ada, grace])
        .await
        .expect("members");
    let mut empty = postio_model::ContactGroup::new("Nobody here", Utc::now());
    groups.create(&mut empty).await.expect("empty");

    let rows = contacts.export_people(&[ada]).await.expect("people");
    let out = contacts.export_groups(&rows).await.expect("groups");
    assert_eq!(out.len(), 1, "a group none of whose members go stays home");
    assert_eq!(out[0].name, "Family");
    assert_eq!(
        out[0].members,
        [rows[0].uid.clone()],
        "only the members exported"
    );
    let again = contacts.export_groups(&rows).await.expect("groups");
    assert_eq!(again[0].uid, out[0].uid, "the group's UID is kept too");
}
