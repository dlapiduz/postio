//! Which way startup routes when several accounts disagree (#183).
//!
//! Every enabled account syncs now, so `startup_route`'s question changed
//! from "is *the* account usable" to "is *any* account usable" — one broken
//! credential must not hold every working account hostage behind the repair
//! screen. No GTK here: the route reads the store and the keyring, and both
//! are in-memory doubles.

use postio_app::{Startup, startup_route};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::{Account, EmailAddress};
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

#[tokio::test]
async fn one_broken_credential_does_not_hold_the_working_account_hostage() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let broken = account(&connection, "Ada", "ada@example.com");
    let working = account(&connection, "Grace", "grace@example.net");
    drop(connection);

    // A password for the second account only: the first routes to repair on
    // its own, and must not when a usable account exists.
    let secrets = MemorySecretStore::new();
    secrets
        .store(
            &AccountKey::new(working.address.address.clone()),
            &Password::new("hunter2"),
        )
        .await
        .expect("a stored password");

    match startup_route(&database, &secrets).await {
        Startup::Ready(account) => assert_eq!(
            account.id, working.id,
            "ready, but for the account with no password"
        ),
        Startup::Onboard(_) => panic!(
            "an account with a working credential was sent to the repair \
             screen because a *different* account is broken"
        ),
    }
    let _ = broken;
}

#[tokio::test]
async fn no_usable_account_still_routes_to_repair() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let first = account(&connection, "Ada", "ada@example.com");
    account(&connection, "Grace", "grace@example.net");
    drop(connection);

    match startup_route(&database, &MemorySecretStore::new()).await {
        Startup::Onboard(Some(repairing)) => assert_eq!(
            repairing.id, first.id,
            "repair opens on the first broken account -- the one most likely \
             just typed"
        ),
        other => panic!("two accounts with no passwords must route to repair: {other:?}"),
    }
}
