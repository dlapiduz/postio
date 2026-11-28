use super::executor::at;
use chrono::Utc;
use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::Connection;
use postio_storage::repository::{AccountRepository, MessageRepository};
use postio_storage::test_support;

async fn message(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    subject: &str,
    received_at: chrono::DateTime<Utc>,
) -> Message {
    let mut message = Message::new(account.id, mailbox, received_at);
    message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    message.subject = Some(subject.to_string());
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create message");
    message
}

struct World {
    _database: postio_storage::Store,
    connection: postio_storage::Checkout,
    work: postio_model::Account,
    home: postio_model::Account,
    work_inbox_message: Message,
    home_inbox_message: Message,
    work_archive_message: Message,
}

async fn named_account(
    connection: &Connection,
    display_name: &str,
    address: &str,
) -> postio_model::Account {
    let mut account =
        postio_model::Account::new(display_name, EmailAddress::new(Some(display_name), address));
    account.incoming.host = "imap.example.com".to_owned();
    account.outgoing.host = "smtp.example.com".to_owned();
    AccountRepository::new(connection)
        .create(&mut account)
        .await
        .expect("create an account");
    account
}

async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");

    let work = named_account(&connection, "Work", "ada@work.example").await;
    let work_inbox = test_support::mailbox(&connection, &work, "INBOX").await.id;
    let work_archive = test_support::mailbox(&connection, &work, "Archive")
        .await
        .id;
    let home = named_account(&connection, "Home", "ada@home.example").await;
    let home_inbox = test_support::mailbox(&connection, &home, "INBOX").await.id;

    let work_inbox_message =
        message(&connection, &work, work_inbox, "Quarterly report", at(9)).await;
    let work_archive_message =
        message(&connection, &work, work_archive, "Quarterly summary", at(8)).await;
    let home_inbox_message =
        message(&connection, &home, home_inbox, "Quarterly bills", at(7)).await;

    World {
        _database: database,
        connection,
        work,
        home,
        work_inbox_message,
        home_inbox_message,
        work_archive_message,
    }
}

async fn run(
    world: &World,
    query: &str,
    scope: Scope,
    account: AccountScope,
) -> Vec<postio_model::MessageId> {
    let parsed = parse(query, at(12).date_naive());
    let request = SearchRequest {
        account,
        query: &parsed,
        scope,
        limit: 50,
        order: postio_search::ResultOrder::Relevance,
    };
    search(&world.connection, &request, at(12))
        .await
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect()
}

#[tokio::test]
async fn the_same_query_string_means_the_same_thing_in_both_scopes() {
    let world = world().await;

    let unified = run(&world, "quarterly", Scope::AllMail, AccountScope::Unified).await;
    assert_eq!(
        unified.len(),
        3,
        "unified search must reach every account's mail: {unified:?}"
    );
    assert!(unified.contains(&world.work_inbox_message.id));
    assert!(unified.contains(&world.home_inbox_message.id));

    let scoped = run(
        &world,
        "quarterly",
        Scope::AllMail,
        AccountScope::Account(world.work.id),
    )
    .await;
    assert_eq!(
        scoped.len(),
        2,
        "an account-scoped search sees only that account: {scoped:?}"
    );
    assert!(scoped.contains(&world.work_inbox_message.id));
    assert!(scoped.contains(&world.work_archive_message.id));
    assert!(
        !scoped.contains(&world.home_inbox_message.id),
        "the other account's mail leaked into an account-scoped search"
    );
}

#[tokio::test]
async fn a_role_scope_and_an_account_scope_compose() {
    let world = world().await;

    let both = run(
        &world,
        "quarterly",
        Scope::Inbox,
        AccountScope::Account(world.work.id),
    )
    .await;
    assert_eq!(
        both,
        vec![world.work_inbox_message.id],
        "'this account's inbox' has to be one query, not a choice between two \
         scopes"
    );

    let every_inbox = run(&world, "quarterly", Scope::Inbox, AccountScope::Unified).await;
    assert_eq!(every_inbox.len(), 2, "{every_inbox:?}");
    assert!(every_inbox.contains(&world.work_inbox_message.id));
    assert!(every_inbox.contains(&world.home_inbox_message.id));
    assert!(
        !every_inbox.contains(&world.work_archive_message.id),
        "the archived message is not in anybody's inbox"
    );
}

#[tokio::test]
async fn the_account_operator_pins_a_search_to_one_account_from_inside_the_query() {
    let world = world().await;

    let by_name = run(
        &world,
        &format!("quarterly account:{}", world.work.display_name),
        Scope::AllMail,
        AccountScope::Unified,
    )
    .await;
    assert_eq!(
        by_name.len(),
        2,
        "`account:` did not narrow a unified search: {by_name:?}"
    );
    assert!(!by_name.contains(&world.home_inbox_message.id));

    let by_address = run(
        &world,
        &format!("quarterly account:{}", world.home.address.address),
        Scope::AllMail,
        AccountScope::Unified,
    )
    .await;
    assert_eq!(by_address, vec![world.home_inbox_message.id]);
}

#[tokio::test]
async fn a_negated_account_operator_means_every_other_account() {
    let world = world().await;

    let others = run(
        &world,
        &format!("quarterly -account:{}", world.work.display_name),
        Scope::AllMail,
        AccountScope::Unified,
    )
    .await;
    assert_eq!(others, vec![world.home_inbox_message.id]);
}

#[tokio::test]
async fn an_account_that_names_nothing_matches_nothing_rather_than_everything() {
    // An unresolvable name must not silently drop the predicate, which would turn `account:typo`
    // into an unscoped search (ADR 0005 Q10).
    let world = world().await;

    let nothing = run(
        &world,
        "quarterly account:nosuchaccount",
        Scope::AllMail,
        AccountScope::Unified,
    )
    .await;
    assert!(
        nothing.is_empty(),
        "an unresolvable account: matched {} messages instead of none",
        nothing.len()
    );
}

/// `Unified` is every enabled account: removing the account predicate alone would reach a disabled
/// account's mail with no row saying where it came from (ADR 0005 Q10, #961).
#[tokio::test]
async fn a_disabled_account_is_not_searched_under_the_unified_scope() {
    let world = world().await;
    world
        .connection
        .execute(
            "UPDATE accounts SET enabled = 0 WHERE id = ?1",
            [world.home.id.get()],
        )
        .await
        .expect("disable the home account");

    let unified = run(&world, "quarterly", Scope::AllMail, AccountScope::Unified).await;

    assert!(
        !unified.contains(&world.home_inbox_message.id),
        "the home account is disabled, so its mail is not in a unified search: \
         {unified:?}"
    );
    assert_eq!(
        unified.len(),
        2,
        "and the enabled account's two messages are still both there: {unified:?}"
    );
}

#[tokio::test]
async fn a_pending_deletion_account_is_not_searched_either() {
    let world = world().await;
    world
        .connection
        .execute(
            "UPDATE accounts SET pending_deletion = 1 WHERE id = ?1",
            [world.home.id.get()],
        )
        .await
        .expect("mark the home account for deletion");

    let unified = run(&world, "quarterly", Scope::AllMail, AccountScope::Unified).await;

    assert!(
        !unified.contains(&world.home_inbox_message.id),
        "{unified:?}"
    );
}

#[tokio::test]
async fn naming_a_disabled_account_explicitly_still_searches_it() {
    // The scope is a view filter, not an authorisation one: settings can still show what a disabled
    // account holds, so the enabled filter belongs to Unified alone.
    let world = world().await;
    world
        .connection
        .execute(
            "UPDATE accounts SET enabled = 0 WHERE id = ?1",
            [world.home.id.get()],
        )
        .await
        .expect("disable the home account");

    let named = run(
        &world,
        "quarterly",
        Scope::AllMail,
        AccountScope::Account(world.home.id),
    )
    .await;

    assert_eq!(named, vec![world.home_inbox_message.id]);
}
