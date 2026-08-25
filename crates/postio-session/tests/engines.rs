//! One engine per enabled account, and a pool sized to serve them (#183).
//!
//! ADR 0005 Q3: each engine holds a connection from the pool, so the
//! composition root must size the pool from the account count and refuse to
//! start more engines than it can serve — otherwise the tenth account
//! deadlocks waiting for a connection a sync pass is holding, which is a
//! hang with no sentence attached. These are the pure halves of that: the
//! arithmetic, and which accounts get engines at all.

use postio_model::{Account, EmailAddress};
use postio_session::{UI_RESERVED_CONNECTIONS, enabled_accounts, engine_budget, pool_size_for};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

fn account(connection: &postio_storage::PooledConnection, name: &str, address: &str) -> Account {
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.incoming.host = "imap.example.com".to_owned();
    account.outgoing.host = "smtp.example.com".to_owned();
    AccountRepository::new(connection)
        .create(&mut account)
        .expect("an account");
    account
}

#[test]
fn the_pool_is_sized_from_the_account_count() {
    // One connection per engine, plus the UI's reserve -- the reads the
    // window makes while every engine is mid-pass.
    assert_eq!(pool_size_for(0), UI_RESERVED_CONNECTIONS + 1);
    assert_eq!(pool_size_for(1), UI_RESERVED_CONNECTIONS + 1);
    assert_eq!(pool_size_for(10), UI_RESERVED_CONNECTIONS + 10);
}

#[test]
fn one_account_costs_what_the_old_default_cost() {
    // "Existing single-account behaviour unchanged": the constant this
    // replaces was postio_storage::db::DEFAULT_MAX_CONNECTIONS = 4, so a
    // one-account store must not suddenly hold more or fewer connections.
    assert_eq!(pool_size_for(1), postio_storage::DEFAULT_MAX_CONNECTIONS);
}

#[test]
fn the_budget_is_what_the_pool_can_serve_beyond_the_ui() {
    assert_eq!(engine_budget(pool_size_for(1)), 1);
    assert_eq!(engine_budget(pool_size_for(10)), 10);
    // A pool too small to reserve the UI's share serves no engines rather
    // than serving them by starving the window.
    assert_eq!(engine_budget(UI_RESERVED_CONNECTIONS), 0);
    assert_eq!(engine_budget(0), 0);
}

#[test]
fn only_enabled_accounts_get_engines() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let ada = account(&connection, "Ada", "ada@example.com");
    let mut grace = account(&connection, "Grace", "grace@example.net");
    grace.enabled = false;
    AccountRepository::new(&connection)
        .update(&mut grace)
        .expect("disable");
    drop(connection);

    let accounts = enabled_accounts(&database);
    assert_eq!(
        accounts
            .iter()
            .map(|account| account.id)
            .collect::<Vec<_>>(),
        vec![ada.id],
        "a disabled account must not cost a connection or open a socket"
    );
}
