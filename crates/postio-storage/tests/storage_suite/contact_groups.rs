//! Contact groups: a named set of people, CRUD over `contact_groups` and
//! `contact_group_members` (specs/005-contacts FR-040). Groups are shared
//! across accounts, as people are.

use chrono::{TimeZone, Utc};

use postio_model::{ContactGroup, ContactId, EmailAddress};
use postio_storage::Connection;
use postio_storage::repository::{ContactGroupRepository, ContactRepository};
use postio_storage::test_support;

fn at(days: i64) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

async fn person(connection: &Connection, name: &str, email: &str) -> ContactId {
    ContactRepository::new(connection)
        .create(Some(name), &[EmailAddress::new(None::<String>, email)])
        .await
        .unwrap_or_else(|e| panic!("create {name}: {e}"))
}

#[tokio::test]
async fn a_group_can_be_created_looked_up_and_renamed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let groups = ContactGroupRepository::new(&connection);

    let mut group = ContactGroup::new("Book club", at(0));
    let id = groups.create(&mut group).await.expect("create");
    assert_eq!(group.id, id, "the id is written back into the value");

    let stored = groups.get(id).await.expect("get").expect("the group");
    assert_eq!(stored.name, "Book club");

    groups.set_name(id, "Reading group").await.expect("rename");
    let renamed = groups.get(id).await.expect("get").expect("the group");
    assert_eq!(renamed.name, "Reading group");
}

#[tokio::test]
async fn getting_a_missing_group_is_none_not_an_error() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let groups = ContactGroupRepository::new(&connection);

    assert!(
        groups
            .get(postio_model::ContactGroupId::new(9999))
            .await
            .expect("get")
            .is_none()
    );
}

#[tokio::test]
async fn members_can_be_added_and_removed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let groups = ContactGroupRepository::new(&connection);
    let ada = person(&connection, "Ada", "ada@example.com").await;
    let grace = person(&connection, "Grace", "grace@example.com").await;

    let mut group = ContactGroup::new("Book club", at(0));
    let group_id = groups.create(&mut group).await.expect("create group");

    groups.add_member(group_id, ada).await.expect("add ada");
    groups.add_member(group_id, grace).await.expect("add grace");

    let members = groups.members(group_id).await.expect("members");
    let mut addresses: Vec<&str> = members
        .iter()
        .map(|c| c.addresses[0].address.address.as_str())
        .collect();
    addresses.sort_unstable();
    assert_eq!(addresses, ["ada@example.com", "grace@example.com"]);

    groups
        .remove_member(group_id, ada)
        .await
        .expect("remove ada");
    let members = groups.members(group_id).await.expect("members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].id, grace);
}

#[tokio::test]
async fn adding_the_same_member_twice_is_not_an_error_and_not_a_duplicate() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let groups = ContactGroupRepository::new(&connection);
    let ada = person(&connection, "Ada", "ada@example.com").await;
    let mut group = ContactGroup::new("Book club", at(0));
    let group_id = groups.create(&mut group).await.expect("create group");

    groups.add_member(group_id, ada).await.expect("add once");
    groups.add_member(group_id, ada).await.expect("add again");

    assert_eq!(groups.members(group_id).await.expect("members").len(), 1);
}

#[tokio::test]
async fn deleting_a_group_leaves_its_members_intact() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let groups = ContactGroupRepository::new(&connection);
    let ada = person(&connection, "Ada", "ada@example.com").await;
    let mut group = ContactGroup::new("Book club", at(0));
    let group_id = groups.create(&mut group).await.expect("create group");
    groups.add_member(group_id, ada).await.expect("add ada");

    assert!(groups.delete(group_id).await.expect("delete"));
    assert!(groups.get(group_id).await.expect("get").is_none());
    assert!(!groups.delete(group_id).await.expect("delete again"));

    // Deleting a group is not deleting the people in it.
    assert!(
        ContactRepository::new(&connection)
            .get(ada)
            .await
            .expect("get")
            .is_some()
    );
}

#[tokio::test]
async fn every_group_is_listed_by_name_whatever_the_account() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let groups = ContactGroupRepository::new(&connection);

    for name in ["zeta", "Alpha", "mid"] {
        groups
            .create(&mut ContactGroup::new(name, at(0)))
            .await
            .expect("create");
    }

    let listed = groups.list().await.expect("list");
    assert_eq!(
        listed.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
        ["Alpha", "mid", "zeta"],
        "by name, ignoring case"
    );
}

#[tokio::test]
async fn a_deleted_member_is_not_expanded_but_keeps_its_membership() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let groups = ContactGroupRepository::new(&connection);
    let ada = person(&connection, "Ada", "ada@example.com").await;
    let grace = person(&connection, "Grace", "grace@example.com").await;
    let group_id = groups
        .create(&mut ContactGroup::new("Book club", at(0)))
        .await
        .expect("create group");
    groups.add_member(group_id, ada).await.expect("add ada");
    groups.add_member(group_id, grace).await.expect("add grace");

    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET state = 'deleted' WHERE id = ?1",
        [ada.get()],
    )
    .await
    .expect("delete ada");

    let expanded = groups.members(group_id).await.expect("members");
    assert_eq!(expanded.iter().map(|c| c.id).collect::<Vec<_>>(), [grace]);
    let kept = postio_storage::sql::scalar(
        &connection,
        "SELECT count(*) FROM contact_group_members WHERE group_id = ?1",
        [group_id.get()],
    )
    .await
    .expect("count members");
    assert_eq!(
        kept, 2,
        "restoring ada will return her to the group (FR-023a)"
    );
}
