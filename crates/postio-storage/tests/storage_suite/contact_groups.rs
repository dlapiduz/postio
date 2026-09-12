//! Contact groups: a named set of contacts, CRUD over `contact_groups` and
//! `contact_group_members` (ADR 0007 Q3).

use chrono::{TimeZone, Utc};

use postio_model::{ContactGroup, EmailAddress};
use postio_storage::repository::{ContactGroupRepository, ContactRepository};
use postio_storage::test_support;

fn at(days: i64) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

#[tokio::test]
async fn a_group_can_be_created_looked_up_and_renamed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let groups = ContactGroupRepository::new(&connection);

    let mut group = ContactGroup::new(Some(account.id), "Book club", at(0));
    let id = groups.create(&mut group).await.expect("create");
    assert_eq!(group.id, id, "the id is written back into the value");

    let stored = groups.get(id).await.expect("get").expect("the group");
    assert_eq!(stored.name, "Book club");
    assert_eq!(stored.account_id, Some(account.id));

    groups.set_name(id, "Reading group").await.expect("rename");
    let renamed = groups.get(id).await.expect("get").expect("the group");
    assert_eq!(renamed.name, "Reading group");
}

#[tokio::test]
async fn a_shared_group_belongs_to_no_account() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let groups = ContactGroupRepository::new(&connection);

    let mut group = ContactGroup::new(None, "Family", at(0));
    groups.create(&mut group).await.expect("create");

    let stored = groups.get(group.id).await.expect("get").expect("the group");
    assert_eq!(stored.account_id, None);
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
    let account = test_support::account(&connection).await;
    let contacts = ContactRepository::new(&connection);
    let groups = ContactGroupRepository::new(&connection);

    let ada = contacts
        .create(
            Some(account.id),
            &EmailAddress::new(Some("Ada"), "ada@example.com"),
            None,
        )
        .await
        .expect("create ada");
    let grace = contacts
        .create(
            Some(account.id),
            &EmailAddress::new(Some("Grace"), "grace@example.com"),
            None,
        )
        .await
        .expect("create grace");

    let mut group = ContactGroup::new(Some(account.id), "Book club", at(0));
    let group_id = groups.create(&mut group).await.expect("create group");

    groups.add_member(group_id, ada).await.expect("add ada");
    groups.add_member(group_id, grace).await.expect("add grace");

    let members = groups.members(group_id).await.expect("members");
    let mut addresses: Vec<&str> = members.iter().map(|c| c.address.address.as_str()).collect();
    addresses.sort_unstable();
    assert_eq!(addresses, ["ada@example.com", "grace@example.com"]);

    groups.remove_member(group_id, ada).await.expect("remove ada");
    let members = groups.members(group_id).await.expect("members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].id, grace);
}

#[tokio::test]
async fn adding_the_same_member_twice_is_not_an_error_and_not_a_duplicate() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let contacts = ContactRepository::new(&connection);
    let groups = ContactGroupRepository::new(&connection);

    let ada = contacts
        .create(
            Some(account.id),
            &EmailAddress::new(Some("Ada"), "ada@example.com"),
            None,
        )
        .await
        .expect("create ada");
    let mut group = ContactGroup::new(Some(account.id), "Book club", at(0));
    let group_id = groups.create(&mut group).await.expect("create group");

    groups.add_member(group_id, ada).await.expect("add once");
    groups.add_member(group_id, ada).await.expect("add again");

    assert_eq!(groups.members(group_id).await.expect("members").len(), 1);
}

#[tokio::test]
async fn deleting_a_group_leaves_its_members_intact() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let contacts = ContactRepository::new(&connection);
    let groups = ContactGroupRepository::new(&connection);

    let ada = contacts
        .create(
            Some(account.id),
            &EmailAddress::new(Some("Ada"), "ada@example.com"),
            None,
        )
        .await
        .expect("create ada");
    let mut group = ContactGroup::new(Some(account.id), "Book club", at(0));
    let group_id = groups.create(&mut group).await.expect("create group");
    groups.add_member(group_id, ada).await.expect("add ada");

    assert!(groups.delete(group_id).await.expect("delete"));
    assert!(groups.get(group_id).await.expect("get").is_none());
    assert!(!groups.delete(group_id).await.expect("delete again"));

    // The contact itself is untouched -- deleting a group is not deleting
    // the people in it.
    assert!(contacts.get(ada).await.expect("get").is_some());
}

#[tokio::test]
async fn listing_groups_matches_by_account_exactly_like_contacts_does() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let groups = ContactGroupRepository::new(&connection);

    let mut mine = ContactGroup::new(Some(account.id), "Mine", at(0));
    groups.create(&mut mine).await.expect("create");
    let mut shared = ContactGroup::new(None, "Shared", at(0));
    groups.create(&mut shared).await.expect("create");

    let listed = groups.list(Some(account.id)).await.expect("list");
    assert_eq!(
        listed.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
        ["Mine"]
    );

    let listed_shared = groups.list(None).await.expect("list");
    assert_eq!(
        listed_shared
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>(),
        ["Shared"]
    );
}
