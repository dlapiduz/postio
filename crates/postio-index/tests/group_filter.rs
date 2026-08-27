//! `group:` — ADR 0007 Q3: "from or to any member", composing with every
//! other operator the way `list:` and `in:` do.

use chrono::{TimeZone, Utc};

use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::{ContactGroupRepository, ContactRepository, MessageRepository};
use postio_storage::test_support;
use rusqlite::Connection;

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, hour, 0, 0).unwrap()
}

struct World {
    _database: postio_storage::Database,
    connection: postio_storage::PooledConnection,
    from_member: Message,
    to_member: Message,
    unrelated: Message,
}

fn world() -> World {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");

    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, "INBOX").id;

    let contacts = ContactRepository::new(&connection);
    let groups = ContactGroupRepository::new(&connection);

    let ada = contacts
        .create(
            Some(account.id),
            &EmailAddress::new(Some("Ada"), "ada@example.com"),
            None,
        )
        .expect("create ada");
    contacts
        .create(
            Some(account.id),
            &EmailAddress::new(Some("Quinn"), "quinn@example.net"),
            None,
        )
        .expect("create quinn");

    let mut family = postio_model::ContactGroup::new(Some(account.id), "family", at(0));
    groups.create(&mut family).expect("create group");
    groups.add_member(family.id, ada).expect("add ada");

    let mut from_member = Message::new(account.id, inbox, at(9));
    from_member.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    from_member.subject = Some("From Ada".into());
    MessageRepository::new(&connection)
        .create(&mut from_member)
        .expect("create");

    let mut to_member = Message::new(account.id, inbox, at(8));
    to_member.from = vec![EmailAddress::new(Some("Someone"), "someone@example.org")];
    to_member.to = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    to_member.subject = Some("To Ada".into());
    MessageRepository::new(&connection)
        .create(&mut to_member)
        .expect("create");

    let mut unrelated = Message::new(account.id, inbox, at(7));
    unrelated.from = vec![EmailAddress::new(Some("Quinn"), "quinn@example.net")];
    unrelated.subject = Some("From Quinn".into());
    MessageRepository::new(&connection)
        .create(&mut unrelated)
        .expect("create");

    World {
        _database: database,
        connection,
        from_member,
        to_member,
        unrelated,
    }
}

fn run(connection: &Connection, query: &str) -> Vec<postio_model::MessageId> {
    let parsed = parse(query, at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Unified,
        query: &parsed,
        scope: Scope::AllMail,
        limit: 50,
    };
    search(connection, &request, at(12))
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect()
}

#[test]
fn group_matches_a_message_from_or_to_any_member() {
    let world = world();
    let mut hits = run(&world.connection, "group:family");
    hits.sort();
    let mut expected = vec![world.from_member.id, world.to_member.id];
    expected.sort();
    assert_eq!(
        hits, expected,
        "a message from a member and one to a member both match; the one \
         with neither does not"
    );
}

#[test]
fn group_negation_excludes_members() {
    let world = world();
    let hits = run(&world.connection, "-group:family");
    assert_eq!(hits, vec![world.unrelated.id], "everyone outside the group");
}

#[test]
fn an_unknown_group_name_matches_nothing_never_everything() {
    // Same reasoning as `account:` and `in:`: an unresolvable name is an
    // empty set of members, not a dropped predicate.
    let world = world();
    assert!(run(&world.connection, "group:nonexistent").is_empty());
}

#[test]
fn group_composes_with_a_text_search() {
    let world = world();
    let hits = run(&world.connection, "group:family from");
    assert_eq!(
        hits,
        vec![world.from_member.id],
        "group: narrows the same way any other filter does"
    );
}
