//! `group:` — ADR 0007 Q3: "from or to any member", composing with every
//! other operator the way `list:` and `in:` do.

use chrono::{TimeZone, Utc};

use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::Connection;
use postio_storage::repository::{ContactGroupRepository, ContactRepository, MessageRepository};
use postio_storage::test_support;

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, hour, 0, 0).unwrap()
}

struct World {
    _database: postio_storage::Store,
    connection: postio_storage::Checkout,
    from_member: Message,
    to_member: Message,
    unrelated: Message,
}

async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");

    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX")
        .await
        .id;

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
    contacts
        .create(
            Some(account.id),
            &EmailAddress::new(Some("Quinn"), "quinn@example.net"),
            None,
        )
        .await
        .expect("create quinn");

    let mut family = postio_model::ContactGroup::new(Some(account.id), "family", at(0));
    groups.create(&mut family).await.expect("create group");
    groups.add_member(family.id, ada).await.expect("add ada");

    let mut from_member = Message::new(account.id, inbox, at(9));
    from_member.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    from_member.subject = Some("From Ada".into());
    MessageRepository::new(&connection)
        .create(&mut from_member)
        .await
        .expect("create");

    let mut to_member = Message::new(account.id, inbox, at(8));
    to_member.from = vec![EmailAddress::new(Some("Someone"), "someone@example.org")];
    to_member.to = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    to_member.subject = Some("To Ada".into());
    MessageRepository::new(&connection)
        .create(&mut to_member)
        .await
        .expect("create");

    let mut unrelated = Message::new(account.id, inbox, at(7));
    unrelated.from = vec![EmailAddress::new(Some("Quinn"), "quinn@example.net")];
    unrelated.subject = Some("From Quinn".into());
    MessageRepository::new(&connection)
        .create(&mut unrelated)
        .await
        .expect("create");

    World {
        _database: database,
        connection,
        from_member,
        to_member,
        unrelated,
    }
}

async fn run(connection: &Connection, query: &str) -> Vec<postio_model::MessageId> {
    let parsed = parse(query, at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Unified,
        query: &parsed,
        scope: Scope::AllMail,
        limit: 50,
        order: postio_search::ResultOrder::Relevance,
    };
    search(connection, &request, at(12))
        .await
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect()
}

#[tokio::test]
async fn group_matches_a_message_from_or_to_any_member() {
    let world = world().await;
    let mut hits = run(&world.connection, "group:family").await;
    hits.sort();
    let mut expected = vec![world.from_member.id, world.to_member.id];
    expected.sort();
    assert_eq!(
        hits, expected,
        "a message from a member and one to a member both match; the one \
         with neither does not"
    );
}

#[tokio::test]
async fn group_negation_excludes_members() {
    let world = world().await;
    let hits = run(&world.connection, "-group:family").await;
    assert_eq!(hits, vec![world.unrelated.id], "everyone outside the group");
}

#[tokio::test]
async fn an_unknown_group_name_matches_nothing_never_everything() {
    // Same reasoning as `account:` and `in:`: an unresolvable name is an
    // empty set of members, not a dropped predicate.
    let world = world().await;
    assert!(run(&world.connection, "group:nonexistent").await.is_empty());
}

#[tokio::test]
async fn group_composes_with_a_text_search() {
    let world = world().await;
    let hits = run(&world.connection, "group:family from").await;
    assert_eq!(
        hits,
        vec![world.from_member.id],
        "group: narrows the same way any other filter does"
    );
}
