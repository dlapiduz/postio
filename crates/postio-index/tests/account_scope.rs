//! Search across accounts, and the `account:` operator. #186.
//!
//! ADR 0005 Q5's rule is that scope is a filter on the executor and never a
//! change to the query language: **the same query string means the same thing
//! in either scope.** These tests are the differential form of that claim —
//! one string, two scopes, and the difference is only ever which accounts'
//! mail is eligible.
//!
//! # Two orthogonal scopes, not one enum
//!
//! `SearchRequest` carries a role scope (`facets::Scope` — the tri-tab's
//! `AllMail`/`Inbox`/`Lists`) *and* an account scope (`AccountScope`). #186
//! chose that over folding account into the role enum, because one enum holds
//! one value and "Inbox, this account only" would have stopped being
//! expressible. The test that matters for the choice is
//! [`a_role_scope_and_an_account_scope_compose`], which asks for exactly that.

use chrono::{TimeZone, Utc};
use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::{AccountRepository, MessageRepository};
use postio_storage::test_support;
use rusqlite::Connection;

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, hour, 0, 0).unwrap()
}

fn message(
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
        .expect("create message");
    message
}

/// Two accounts, each with an inbox and an archive, each holding one message
/// whose subject carries the same word.
struct World {
    _database: postio_storage::Database,
    connection: postio_storage::PooledConnection,
    work: postio_model::Account,
    home: postio_model::Account,
    work_inbox_message: Message,
    home_inbox_message: Message,
    work_archive_message: Message,
}

/// An account with its own name and address.
///
/// `test_support::account` gives every account the same "Test" /
/// `test@example.com`, which is fine for one and useless for two: half of
/// what these tests assert is that `account:` picks *this* one out.
fn named_account(
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
        .expect("create an account");
    account
}

fn world() -> World {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");

    let work = named_account(&connection, "Work", "ada@work.example");
    let work_inbox = test_support::mailbox(&connection, &work, "INBOX").id;
    let work_archive = test_support::mailbox(&connection, &work, "Archive").id;
    let home = named_account(&connection, "Home", "ada@home.example");
    let home_inbox = test_support::mailbox(&connection, &home, "INBOX").id;

    let work_inbox_message = message(&connection, &work, work_inbox, "Quarterly report", at(9));
    let work_archive_message =
        message(&connection, &work, work_archive, "Quarterly summary", at(8));
    let home_inbox_message = message(&connection, &home, home_inbox, "Quarterly bills", at(7));

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

fn run(
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
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect()
}

#[test]
fn the_same_query_string_means_the_same_thing_in_both_scopes() {
    // ADR 0005 Q5's rule, stated as a difference. Nothing about the string
    // changes; the eligible set does.
    let world = world();

    let unified = run(&world, "quarterly", Scope::AllMail, AccountScope::Unified);
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
    );
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

#[test]
fn a_role_scope_and_an_account_scope_compose() {
    // The reason #186 kept them as two fields. Under a single enum with
    // `Account` as a fourth variant, this query could not be asked at all:
    // one enum holds one value, so "Inbox" and "this account" would have been
    // mutually exclusive.
    let world = world();

    let both = run(
        &world,
        "quarterly",
        Scope::Inbox,
        AccountScope::Account(world.work.id),
    );
    assert_eq!(
        both,
        vec![world.work_inbox_message.id],
        "'this account's inbox' has to be one query, not a choice between two \
         scopes"
    );

    // And the role predicate is unchanged when the account one is absent —
    // "every account's inbox", which is a predicate removal and not a
    // redefinition (ADR 0005 Q5a).
    let every_inbox = run(&world, "quarterly", Scope::Inbox, AccountScope::Unified);
    assert_eq!(every_inbox.len(), 2, "{every_inbox:?}");
    assert!(every_inbox.contains(&world.work_inbox_message.id));
    assert!(every_inbox.contains(&world.home_inbox_message.id));
    assert!(
        !every_inbox.contains(&world.work_archive_message.id),
        "the archived message is not in anybody's inbox"
    );
}

#[test]
fn the_account_operator_pins_a_search_to_one_account_from_inside_the_query() {
    // What makes a saved search portable (ADR 0005 Q12): the string carries
    // the account, so it means the same thing run from any scope.
    let world = world();

    let by_name = run(
        &world,
        &format!("quarterly account:{}", world.work.display_name),
        Scope::AllMail,
        AccountScope::Unified,
    );
    assert_eq!(
        by_name.len(),
        2,
        "`account:` did not narrow a unified search: {by_name:?}"
    );
    assert!(!by_name.contains(&world.home_inbox_message.id));

    // By address too — the name a person is most likely to remember.
    let by_address = run(
        &world,
        &format!("quarterly account:{}", world.home.address.address),
        Scope::AllMail,
        AccountScope::Unified,
    );
    assert_eq!(by_address, vec![world.home_inbox_message.id]);
}

#[test]
fn a_negated_account_operator_means_every_other_account() {
    let world = world();

    let others = run(
        &world,
        &format!("quarterly -account:{}", world.work.display_name),
        Scope::AllMail,
        AccountScope::Unified,
    );
    assert_eq!(others, vec![world.home_inbox_message.id]);
}

#[test]
fn an_account_that_names_nothing_matches_nothing_rather_than_everything() {
    // The failure mode worth naming: an unresolvable name must not silently
    // drop the predicate, which would turn "account:typo" into an unscoped
    // search — the same class of lie ADR 0005 Q10 forbids of aggregate views.
    let world = world();

    let nothing = run(
        &world,
        "quarterly account:nosuchaccount",
        Scope::AllMail,
        AccountScope::Unified,
    );
    assert!(
        nothing.is_empty(),
        "an unresolvable account: matched {} messages instead of none",
        nothing.len()
    );
}
