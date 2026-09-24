//! Joining people, and taking a join apart (specs/005-contacts User Story 2,
//! FR-010..FR-017, research R8).
//!
//! A join keeps everything -- every address, every sighting, every group,
//! every note -- and its undo puts both people back exactly as they were.
//! Detaching gives an address back its own person, with its own history;
//! adding or moving an address never leaves it with two owners.

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{
    Account, ContactGroup, ContactId, ContactSource, ContactState, EmailAddress, MailboxId,
    Message,
};
use postio_storage::Connection;
use postio_storage::repository::{ContactGroupRepository, ContactRepository};
use postio_storage::test_support;

fn at(days: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

fn address(name: Option<&str>, email: &str) -> EmailAddress {
    EmailAddress::new(name, email)
}

async fn setup() -> (postio_storage::Store, postio_storage::Checkout, Account, MailboxId) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (database, connection, account, inbox)
}

/// Records `times` messages from `email`; with `sent`, messages the user
/// sent to it instead.
async fn mail(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
    email: &str,
    times: u32,
    sent: bool,
) {
    for day in 0..times {
        let mut message = Message::new(account.id, inbox, at(i64::from(day)));
        if sent {
            message.from = vec![account.address.clone()];
            message.to = vec![address(Some("Ada"), email)];
        } else {
            message.from = vec![address(Some("Ada"), email)];
        }
        ContactRepository::new(connection)
            .record_message(&message, std::slice::from_ref(&account.address))
            .await
            .expect("record");
    }
}

async fn owner(connection: &Connection, email: &str) -> postio_model::Contact {
    ContactRepository::new(connection)
        .by_address(email)
        .await
        .expect("lookup")
        .unwrap_or_else(|| panic!("nobody owns {email}"))
}

async fn get(connection: &Connection, id: ContactId) -> postio_model::Contact {
    ContactRepository::new(connection)
        .get(id)
        .await
        .expect("get")
        .expect("the person")
}

fn addresses(person: &postio_model::Contact) -> Vec<String> {
    let mut all: Vec<String> = person
        .addresses
        .iter()
        .map(|a| a.address.address.clone())
        .collect();
    all.sort();
    all
}

#[tokio::test]
async fn a_join_keeps_every_address_sighting_group_and_note() {
    let (_db, connection, account, inbox) = setup().await;
    mail(&connection, &account, inbox, "ada@work.example", 3, false).await;
    mail(&connection, &account, inbox, "ada@home.example", 2, true).await;
    let work = owner(&connection, "ada@work.example").await;
    let home = owner(&connection, "ada@home.example").await;
    let contacts = ContactRepository::new(&connection);
    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET note = ?2 WHERE id = ?1",
        postio_storage::sql::bind![home.id.get(), "met at the conference"],
    )
    .await
    .expect("a note");
    let groups = ContactGroupRepository::new(&connection);
    let family = groups
        .create(&mut ContactGroup::new("Family", at(0)))
        .await
        .expect("group");
    groups.add_member(family, home.id).await.expect("member");

    let receipt = contacts
        .join(work.id, &[home.id], "Ada Lovelace", None)
        .await
        .expect("join");

    let ada = get(&connection, work.id).await;
    assert_eq!(addresses(&ada), ["ada@home.example", "ada@work.example"]);
    assert_eq!(ada.times_seen, 5, "both addresses' sightings, summed");
    assert_eq!(ada.written, 2, "written-to survives the join");
    assert_eq!(ada.name.as_deref(), Some("Ada Lovelace"), "every join ends with a name");
    assert_eq!(ada.source, ContactSource::User, "a join is the user's act");
    assert_eq!(ada.note.as_deref(), Some("met at the conference"));
    assert_eq!(
        groups.members(family).await.expect("members").iter().map(|c| c.id).collect::<Vec<_>>(),
        [work.id],
        "the survivor inherits the absorbed person's groups"
    );
    assert_eq!(get(&connection, home.id).await.state, ContactState::Merged);
    assert_eq!(receipt.into, work.id);

    // Mail to either address now counts toward one person.
    mail(&connection, &account, inbox, "ada@home.example", 1, false).await;
    assert_eq!(owner(&connection, "ada@home.example").await.id, work.id);
    assert_eq!(get(&connection, work.id).await.times_seen, 6);
}

#[tokio::test]
async fn unjoining_puts_both_people_back_exactly() {
    let (_db, connection, account, inbox) = setup().await;
    mail(&connection, &account, inbox, "ada@work.example", 3, false).await;
    mail(&connection, &account, inbox, "ada@home.example", 2, false).await;
    let work = owner(&connection, "ada@work.example").await;
    let home = owner(&connection, "ada@home.example").await;
    let groups = ContactGroupRepository::new(&connection);
    let family = groups
        .create(&mut ContactGroup::new("Family", at(0)))
        .await
        .expect("group");
    groups.add_member(family, home.id).await.expect("member");
    let before_work = get(&connection, work.id).await;
    let before_home = get(&connection, home.id).await;
    let contacts = ContactRepository::new(&connection);

    let receipt = contacts
        .join(work.id, &[home.id], "Ada Lovelace", Some("Analytical Engines"))
        .await
        .expect("join");
    contacts.unjoin(&receipt).await.expect("unjoin");

    assert_eq!(get(&connection, work.id).await, before_work, "the survivor, as it was");
    assert_eq!(get(&connection, home.id).await, before_home, "the absorbed, as it was");
    assert_eq!(
        groups.members(family).await.expect("members").iter().map(|c| c.id).collect::<Vec<_>>(),
        [home.id],
        "the membership the join added is taken back, and only that one"
    );
}

#[tokio::test]
async fn detaching_an_address_gives_it_a_person_of_its_own_with_its_history() {
    let (_db, connection, account, inbox) = setup().await;
    mail(&connection, &account, inbox, "ada@work.example", 3, false).await;
    mail(&connection, &account, inbox, "ada@old.example", 4, false).await;
    let work = owner(&connection, "ada@work.example").await;
    let old = owner(&connection, "ada@old.example").await;
    let contacts = ContactRepository::new(&connection);
    contacts
        .join(work.id, &[old.id], "Ada", None)
        .await
        .expect("join");
    let old_address = owner(&connection, "ada@old.example")
        .await
        .addresses
        .into_iter()
        .find(|a| a.address.address == "ada@old.example")
        .expect("the old address")
        .id;

    let detached = contacts.detach_address(old_address).await.expect("detach");

    let split = get(&connection, detached).await;
    assert_ne!(detached, work.id);
    assert_eq!(addresses(&split), ["ada@old.example"]);
    assert_eq!(split.times_seen, 4, "it carries its own sighting history");
    assert_eq!(get(&connection, work.id).await.times_seen, 3, "and the other keeps its own");

    let last = get(&connection, work.id).await.addresses[0].id;
    let refused = contacts.detach_address(last).await;
    assert!(refused.is_err(), "a person keeps at least one address -- delete instead");
}

#[tokio::test]
async fn releasing_an_added_address_leaves_it_nobodys() {
    let (_db, connection, _account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .create(Some("Ada"), &[address(None, "ada@example.com")])
        .await
        .expect("ada");
    let before = get(&connection, ada).await;
    let added = contacts
        .add_address(ada, &address(None, "ada@home.example"))
        .await
        .expect("add");

    let released = contacts.release_address(added).await.expect("release");

    assert_eq!(released.previous, Some(ada));
    assert_eq!(released.emptied, None, "ada still has her first address");
    assert_eq!(get(&connection, ada).await, before, "ada, as before the add");
    assert!(
        contacts.by_address("ada@home.example").await.expect("lookup").is_none(),
        "nobody owns the released address"
    );
}

#[tokio::test]
async fn an_address_is_never_owned_twice() {
    let (_db, connection, account, inbox) = setup().await;
    mail(&connection, &account, inbox, "grace@example.org", 1, false).await;
    let grace = owner(&connection, "grace@example.org").await;
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .create(Some("Ada"), &[address(None, "ada@example.com")])
        .await
        .expect("create ada");

    let refused = contacts
        .add_address(ada, &address(None, "Grace@Example.org"))
        .await;
    match refused {
        Err(postio_storage::Error::AddressOwned { owner, .. }) => {
            assert_eq!(owner, grace.id.get(), "the refusal names who has it (FR-015)")
        }
        other => panic!("expected AddressOwned, got {other:?}"),
    }

    let new = contacts
        .add_address(ada, &address(None, "ada@home.example"))
        .await
        .expect("an address nobody owns");
    assert!(get(&connection, ada).await.addresses.iter().any(|a| a.id == new));

    // "Move it": the owner lets go, and a person left with nothing is folded
    // away -- kept, so an undo has someone to give the address back to.
    let grace_address = grace.addresses[0].id;
    let moved = contacts
        .move_address(grace_address, ada, None)
        .await
        .expect("move");
    assert_eq!(moved.previous, Some(grace.id));
    assert_eq!(moved.emptied, Some(ContactState::Live), "grace was live");
    assert_eq!(owner(&connection, "grace@example.org").await.id, ada);
    assert_eq!(get(&connection, grace.id).await.state, ContactState::Merged);

    // And undoing the move puts grace back exactly.
    contacts
        .move_address(grace_address, grace.id, moved.emptied)
        .await
        .expect("move back");
    assert_eq!(get(&connection, grace.id).await, grace, "grace, as she was");
}

#[tokio::test]
async fn a_moved_address_lifts_its_suppression() {
    let (_db, connection, account, inbox) = setup().await;
    mail(&connection, &account, inbox, "robot@example.com", 5, false).await;
    let robot = owner(&connection, "robot@example.com").await;
    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET state = 'deleted' WHERE id = ?1",
        [robot.id.get()],
    )
    .await
    .expect("delete");
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .create(Some("Ada"), &[address(None, "ada@example.com")])
        .await
        .expect("create ada");

    contacts
        .move_address(robot.addresses[0].id, ada, None)
        .await
        .expect("move from a deleted person");

    let now = owner(&connection, "robot@example.com").await;
    assert_eq!(now.id, ada);
    assert_eq!(now.state, ContactState::Live, "offered again (FR-024)");
    assert_eq!(now.times_seen, 5, "with its history");
}

#[tokio::test]
async fn the_preferred_address_is_one_of_the_persons_own() {
    let (_db, connection, _account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .create(
            Some("Ada"),
            &[address(None, "ada@work.example"), address(None, "ada@home.example")],
        )
        .await
        .expect("ada");
    let grace = contacts
        .create(Some("Grace"), &[address(None, "grace@example.org")])
        .await
        .expect("grace");
    let home = get(&connection, ada)
        .await
        .addresses
        .into_iter()
        .find(|a| a.address.address == "ada@home.example")
        .expect("home")
        .id;

    let previous = contacts.set_preferred(ada, home).await.expect("prefer home");
    assert_ne!(previous, home);
    assert_eq!(get(&connection, ada).await.preferred, home);
    assert_eq!(
        contacts.complete("ada", 8).await.expect("complete")[0].addresses[0]
            .address
            .address,
        "ada@home.example",
        "completion offers the preferred address first (FR-016)"
    );

    let graces = get(&connection, grace).await.addresses[0].id;
    assert!(
        contacts.set_preferred(ada, graces).await.is_err(),
        "someone else's address cannot be ada's preferred one"
    );
}
