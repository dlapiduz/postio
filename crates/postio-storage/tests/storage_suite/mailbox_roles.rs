//! The per-account role map (ADR 0025): which of an account's server folders
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
