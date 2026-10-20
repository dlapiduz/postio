//! `with:` -- from, to, cc or bcc is any of the listed addresses, by exact
//! address (specs/005-contacts R6, contracts/query-with.md).

use chrono::{TimeZone, Utc};

use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, EmailAddress, Message, MessageId};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, hour, 0, 0).unwrap()
}

fn address(email: &str) -> EmailAddress {
    EmailAddress::new(None::<String>, email)
}

struct World {
    _database: postio_storage::Store,
    connection: postio_storage::Checkout,
    from_work: MessageId,
    to_home_cc: MessageId,
    bcc_work: MessageId,
    named_only: MessageId,
    unrelated: MessageId,
}

async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let store = async |build: &dyn Fn(&mut Message), hour: u32| {
        let mut message = Message::new(account.id, inbox, at(hour));
        message.from = vec![address("someone@example.org")];
        build(&mut message);
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("store");
        message.id
    };

    let from_work = store(&|m| m.from = vec![address("Ada@Work.example")], 9).await;
    let to_home_cc = store(&|m| m.cc = vec![address("ada@home.example")], 8).await;
    let bcc_work = store(&|m| m.bcc = vec![address("ada@work.example")], 7).await;
    // The string appears only in a display name: not an address match.
    let named_only = store(
        &|m| {
            m.to = vec![EmailAddress::new(
                Some("ada@work.example fan"),
                "fan@example.net",
            )]
        },
        6,
    )
    .await;
    let unrelated = store(&|m| m.to = vec![address("grace@example.org")], 5).await;

    World {
        _database: database,
        connection,
        from_work,
        to_home_cc,
        bcc_work,
        named_only,
        unrelated,
    }
}

async fn run(connection: &Connection, query: &str) -> Vec<MessageId> {
    let parsed = parse(query, at(12).date_naive());
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
async fn with_matches_any_listed_address_in_any_address_header() {
    let world = world().await;
    assert_eq!(
        run(&world.connection, "with:ada@work.example,ada@home.example").await,
        sorted(vec![world.from_work, world.to_home_cc, world.bcc_work]),
        "from, cc and bcc all count; case does not"
    );
}

#[tokio::test]
async fn a_display_name_containing_the_address_is_not_a_match() {
    let world = world().await;
    let hits = run(&world.connection, "with:ada@work.example").await;
    assert!(
        !hits.contains(&world.named_only),
        "exact address, not full text"
    );
    assert_eq!(hits, sorted(vec![world.from_work, world.bcc_work]));
}

#[tokio::test]
async fn an_address_the_store_has_never_seen_matches_nothing_never_everything() {
    let world = world().await;
    assert!(
        run(&world.connection, "with:nobody@example.invalid")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn two_with_tokens_mean_mail_involving_both() {
    let world = world().await;
    assert!(
        run(
            &world.connection,
            "with:ada@work.example with:grace@example.org"
        )
        .await
        .is_empty(),
        "no message involves both"
    );
    let _ = world.unrelated;
}

#[tokio::test]
async fn negated_with_is_everything_else() {
    let world = world().await;
    assert_eq!(
        run(&world.connection, "-with:ada@work.example,ada@home.example").await,
        sorted(vec![world.named_only, world.unrelated])
    );
}
