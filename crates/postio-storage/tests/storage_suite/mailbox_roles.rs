//! The per-account role map (ADR 0035): which of an account's server folders
//! plays each role, as the user chose it. One row per `(account, role)`, keyed
//! by path so the choice survives the folder's row being retired and
//! re-created, and gone with the account.

use rusqlite::Connection;

use postio_model::{Account, AccountId, EmailAddress, MailboxRole};
use postio_storage::repository::{AccountRepository, MailboxRoleRepository};
use postio_storage::test_support;

fn an_account(connection: &Connection, address: &str) -> AccountId {
    let mut account = Account::new("Test", EmailAddress::new(None::<String>, address));
    AccountRepository::new(connection)
        .create(&mut account)
        .expect("create an account")
}

#[test]
fn a_role_maps_to_one_path_per_account_and_a_second_choice_replaces_the_first() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection, "ada@example.com");
    let roles = MailboxRoleRepository::new(&connection);

    roles
        .set(account, MailboxRole::Sent, "Sent")
        .expect("map sent");
    roles
        .set(account, MailboxRole::Sent, "Sent Messages")
        .expect("map sent again");
    roles
        .set(account, MailboxRole::Archive, "Archives")
        .expect("map archive");

    assert_eq!(
        roles.for_account(account).expect("the map"),
        vec![
            (MailboxRole::Archive, "Archives".to_owned()),
            (MailboxRole::Sent, "Sent Messages".to_owned()),
        ],
        "one folder per role, the later choice replacing the earlier"
    );

    roles
        .clear(account, MailboxRole::Sent)
        .expect("back to automatic");
    assert_eq!(
        roles.for_account(account).expect("the map"),
        vec![(MailboxRole::Archive, "Archives".to_owned())]
    );
}

#[test]
fn an_accounts_map_is_its_own_and_dies_with_it() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let icloud = an_account(&connection, "ada@example.com");
    let other = an_account(&connection, "ada@example.net");
    let roles = MailboxRoleRepository::new(&connection);

    roles
        .set(icloud, MailboxRole::Sent, "Sent Messages")
        .expect("map sent");

    assert!(
        roles.for_account(other).expect("the map").is_empty(),
        "a mapping on one account says nothing about another"
    );

    assert!(
        AccountRepository::new(&connection)
            .delete(icloud)
            .expect("delete")
    );
    assert!(
        roles.for_account(icloud).expect("the map").is_empty(),
        "the map goes with the account"
    );
}

// ── A server's refusal to create a folder (spec 003, FR-031) ────────────────

#[test]
fn a_refusal_is_recorded_per_account_and_role_with_the_servers_own_words() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let ada = an_account(&connection, "ada@example.com");
    let grace = an_account(&connection, "grace@example.net");
    let roles = MailboxRoleRepository::new(&connection);

    roles
        .refuse(ada, MailboxRole::Junk, "Permission denied")
        .expect("refuse");
    roles
        .refuse(grace, MailboxRole::Junk, "Mailbox already exists")
        .expect("refuse");

    // Per account: one server saying no says nothing about another's.
    assert_eq!(
        roles.refusals(ada).expect("refusals"),
        vec![(MailboxRole::Junk, "Permission denied".to_owned())]
    );
    assert_eq!(
        roles.refusals(grace).expect("refusals"),
        vec![(MailboxRole::Junk, "Mailbox already exists".to_owned())]
    );
}

#[test]
fn refusing_the_same_role_twice_keeps_the_current_reason_not_a_second_row() {
    // What matters is the answer the server is giving now, not how many times
    // it has given one.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection, "ada@example.com");
    let roles = MailboxRoleRepository::new(&connection);

    roles
        .refuse(account, MailboxRole::Trash, "Permission denied")
        .expect("refuse");
    roles
        .refuse(account, MailboxRole::Trash, "Quota exceeded")
        .expect("refuse again");

    assert_eq!(
        roles.refusals(account).expect("refusals"),
        vec![(MailboxRole::Trash, "Quota exceeded".to_owned())]
    );
}

#[test]
fn mapping_a_role_by_hand_clears_the_refusal_that_was_standing_against_it() {
    // The user has answered the question another way, so the record of the
    // server saying no is stale -- and leaving it would suppress an attempt
    // nobody is going to make anyway, on a role that now resolves.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection, "ada@example.com");
    let roles = MailboxRoleRepository::new(&connection);

    roles
        .refuse(account, MailboxRole::Archive, "Permission denied")
        .expect("refuse");
    roles
        .clear(account, MailboxRole::Archive)
        .expect("the user maps it somewhere, or back to automatic");

    assert!(
        roles.refusals(account).expect("refusals").is_empty(),
        "the refusal outlived the question it was answering"
    );
}

#[test]
fn a_refusal_goes_with_its_account() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = an_account(&connection, "ada@example.com");
    let roles = MailboxRoleRepository::new(&connection);
    roles
        .refuse(account, MailboxRole::Sent, "Permission denied")
        .expect("refuse");

    AccountRepository::new(&connection)
        .delete(account)
        .expect("remove the account");

    let orphans: i64 = connection
        .query_row("SELECT count(*) FROM mailbox_role_refusals", [], |row| {
            row.get(0)
        })
        .expect("count");
    assert_eq!(orphans, 0, "the refusal outlived the account it was about");
}
