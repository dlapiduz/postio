//! `from:` and `to:` that know people, off unless asked
//! (specs/011-focus-contacts S4, plan 1.3).
//!
//! A person writes `from:lovelace` meaning her, not the words that happen to
//! be in a header. With contact-aware search on, a word that names a contact
//! finds the mail of every address they own, and `from:@Group` finds the mail
//! of every member; off, both mean exactly what they always have, so a
//! frontend that has not adopted people sees the same results as before.

use chrono::{TimeZone, Utc};

use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, EmailAddress, Message, MessageId};
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
    from_work: MessageId,
    from_home: MessageId,
    to_work: MessageId,
    from_grace: MessageId,
    from_stranger: MessageId,
}

async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let store = async |name: Option<&str>, from: &str, to: &str, hour: u32| {
        let mut message = Message::new(account.id, inbox, at(hour));
        message.from = vec![EmailAddress::new(name, from)];
        message.to = vec![EmailAddress::new(None::<String>, to)];
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("store");
        message.id
    };
    // None of the header names say "lovelace": only the contact does.
    let from_work = store(Some("A. L."), "ada@work.example", "me@example.test", 9).await;
    let from_home = store(Some("ada"), "ada@home.example", "me@example.test", 8).await;
    let to_work = store(None, "me@example.test", "ada@work.example", 7).await;
    let from_grace = store(Some("Grace"), "grace@navy.example", "me@example.test", 6).await;
    let from_stranger = store(None, "not-ada@example.net", "me@example.test", 5).await;

    let people = ContactRepository::new(&connection);
    let ada = people
        .create(
            Some("Ada Lovelace"),
            &[
                EmailAddress::new(None::<String>, "ada@work.example"),
                EmailAddress::new(None::<String>, "ada@home.example"),
            ],
        )
        .await
        .expect("ada");
    let grace = people
        .create(
            Some("Grace Hopper"),
            &[EmailAddress::new(None::<String>, "grace@navy.example")],
        )
        .await
        .expect("grace");
    let groups = ContactGroupRepository::new(&connection);
    let mut group = postio_model::ContactGroup::new("Analysts", at(0));
    let group = groups.create(&mut group).await.expect("a group");
    groups.add_member(group, ada).await.expect("ada joins");
    groups.add_member(group, grace).await.expect("grace joins");

    World {
        _database: database,
        connection,
        from_work,
        from_home,
        to_work,
        from_grace,
        from_stranger,
    }
}

async fn run(connection: &Connection, query: &str, aware: bool) -> Vec<MessageId> {
    let mut parsed = parse(query, at(12).date_naive());
    if aware {
        parsed = parsed.contact_aware();
    }
    let request = SearchRequest {
        account: AccountScope::Unified,
        query: &parsed,
        scope: Scope::AllMail,
        limit: 50,
        order: postio_search::ResultOrder::Relevance,
    };
    let mut ids: Vec<MessageId> = search(connection, &request, at(12))
        .await
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<MessageId>) -> Vec<MessageId> {
    ids.sort();
    ids
}

#[tokio::test]
async fn off_from_means_the_words_in_the_header_and_nothing_more() {
    let world = world().await;
    assert!(
        run(&world.connection, "from:lovelace", false)
            .await
            .is_empty()
    );
    assert!(
        run(&world.connection, "from:@Analysts", false)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn on_a_word_that_names_a_contact_finds_mail_from_every_address_they_own() {
    let world = world().await;
    assert_eq!(
        run(&world.connection, "from:lovelace", true).await,
        sorted(vec![world.from_work, world.from_home]),
        "both of her addresses, whatever name the header gave; not the mail to her, \
         and not the stranger"
    );
    assert_eq!(
        run(&world.connection, "to:lovelace", true).await,
        vec![world.to_work],
        "to: reads the same person in the recipients"
    );
}

#[tokio::test]
async fn on_the_words_of_a_header_still_count() {
    let world = world().await;
    assert_eq!(
        run(&world.connection, "from:navy", true).await,
        vec![world.from_grace],
        "an address word is a header match and a contact term alike"
    );
    assert_eq!(
        run(&world.connection, "from:grace", true).await,
        vec![world.from_grace],
        "found once, not twice"
    );
}

#[tokio::test]
async fn on_from_at_a_group_finds_mail_from_every_member() {
    let world = world().await;
    assert_eq!(
        run(&world.connection, "from:@Analysts", true).await,
        sorted(vec![world.from_work, world.from_home, world.from_grace])
    );
    assert_eq!(
        run(&world.connection, "to:@Analysts", true).await,
        vec![world.to_work],
        "to: reads members as recipients"
    );
    assert!(
        run(&world.connection, "from:@Nobody", true)
            .await
            .is_empty(),
        "a group nobody made matches nothing, never everything"
    );
}

#[tokio::test]
async fn on_a_negation_and_a_stranger_behave() {
    let world = world().await;
    let not_ada = run(&world.connection, "-from:lovelace", true).await;
    assert!(not_ada.contains(&world.from_stranger) && !not_ada.contains(&world.from_work));
}
