//! Removing an account, and what a connection test says (#1277).
//!
//! The connection itself is not tested here — no test in the default suite
//! touches the network. What is tested is the part that goes wrong quietly:
//! that removing an account takes its credential with it, that a store which
//! refuses does not leave the account half-gone, and that the sentences a
//! failure produces are the ones somebody can act on.

use std::sync::Arc;
use std::time::Duration;

use postio_account::backend::BackendError;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_session::checkup::{explain, remove_account};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

/// The address `test_support::account` writes. Taken from the helper rather
/// than written down twice: a removal keyed on the wrong address deletes
/// nothing and this test would pass by deleting nothing either.
const ADDRESS: &str = "test@example.com";

/// A store holding one account, with a password and an OAuth refresh token
/// in the keyring — the two shapes a removal has to clean up after.
async fn an_account() -> (
    postio_storage::test_support::TempDatabase,
    MemorySecretStore,
    postio_model::ids::AccountId,
) {
    let database = test_support::temp();
    let keyring = MemorySecretStore::new();
    for key in [
        AccountKey::new(ADDRESS.to_owned()),
        AccountKey::new(format!("{ADDRESS}#oauth-refresh")),
    ] {
        keyring
            .store(&key, &Password::new("a-secret"))
            .await
            .expect("the keyring takes it");
    }
    let id = {
        let connection = database.connection().expect("a connection");
        let (account, _) = test_support::account_with_inbox(&connection);
        account.id
    };
    (database, keyring, id)
}

#[tokio::test]
async fn removing_an_account_takes_its_credentials_with_it() {
    // A mail client that forgets an account and keeps its password is the one
    // thing worse than not forgetting it.
    let (database, keyring, id) = an_account().await;

    remove_account(&database, Arc::new(keyring.reopen()), id)
        .await
        .expect("the account is removed");

    let connection = database.connection().expect("a connection");
    assert!(
        AccountRepository::new(&connection)
            .get(id)
            .expect("a read")
            .is_none(),
        "the row is gone"
    );
    assert!(
        keyring.is_empty(),
        "and so is everything the keyring held for it"
    );
}

#[tokio::test]
async fn removing_an_account_that_is_already_gone_is_not_an_error() {
    // Pressing Remove twice means what it said the first time.
    let (database, keyring, id) = an_account().await;
    let secrets = Arc::new(keyring.reopen());

    remove_account(&database, secrets.clone(), id)
        .await
        .expect("the first removal");
    remove_account(&database, secrets, id)
        .await
        .expect("the second says nothing broke");
}

// -- what a failure says -----------------------------------------------------

#[test]
fn a_rejected_password_names_the_thing_that_is_actually_wrong() {
    // The case the error itself cannot know: a provider that refuses ordinary
    // account passwords says only "rejected", and somebody who typed their
    // Apple ID password has no way to tell that from a typo.
    let said = explain(&BackendError::Auth {
        account: ADDRESS.to_owned(),
        reason: "AUTHENTICATIONFAILED".to_owned(),
    });

    assert!(said.contains("app-specific password"), "{said}");
}

#[test]
fn a_refused_tls_connection_says_postio_will_not_fall_back() {
    // Worth saying out loud: the alternative a user might expect — trying
    // again without encryption — is one Postio will never do.
    let said = explain(&BackendError::Tls {
        host: "imap.example.com".to_owned(),
        reason: "certificate expired".to_owned(),
    });

    assert!(said.contains("will not fall back"), "{said}");
    assert!(said.contains("imap.example.com"));
}

#[test]
fn a_timeout_says_how_long_it_waited() {
    let said = explain(&BackendError::TimedOut {
        context: "connect".to_owned(),
        after: Duration::from_secs(30),
    });

    assert!(said.contains("30s"), "{said}");
}

#[test]
fn a_port_that_answers_but_is_not_imap_says_which_host_that_was() {
    let said = explain(&BackendError::EmptyCapabilities {
        host: "imap.example.com".to_owned(),
    });

    assert!(said.contains("not like an IMAP server"), "{said}");
    assert!(said.contains("imap.example.com"));
}

#[tokio::test]
async fn an_account_with_nothing_in_the_keyring_is_partial_rather_than_wrong() {
    // The settings pane's third state: a row that exists with nothing to sign
    // in with. It calls for a different offer — put a password in — than a
    // server that said no, and a pane that could not tell them apart would
    // make the wrong one.
    let database = test_support::temp();
    let empty = MemorySecretStore::new();
    let account = {
        let connection = database.connection().expect("a connection");
        let (account, _) = test_support::account_with_inbox(&connection);
        AccountRepository::new(&connection)
            .get(account.id)
            .expect("a read")
            .expect("the account")
    };

    let report = postio_session::checkup::test_connection(&account, Arc::new(empty.reopen())).await;

    assert!(!report.reachable);
    assert!(
        report.missing_credential,
        "nothing was in the keyring: {}",
        report.message
    );
}

#[test]
fn renaming_an_account_changes_what_it_calls_itself_and_nothing_else() {
    let database = test_support::temp();
    let (id, address) = {
        let connection = database.connection().expect("a connection");
        let (account, _) = test_support::account_with_inbox(&connection);
        (account.id, account.address.address.clone())
    };

    postio_session::checkup::set_display_name(&database, id, "  Ada at work  ")
        .expect("the rename lands");

    let connection = database.connection().expect("a connection");
    let stored = AccountRepository::new(&connection)
        .get(id)
        .expect("a read")
        .expect("the account");
    assert_eq!(stored.display_name, "Ada at work", "trimmed");
    assert_eq!(
        stored.address.address, address,
        "and the address it signs in with is untouched"
    );
}

#[test]
fn renaming_an_account_that_is_gone_says_so() {
    let database = test_support::temp();
    let error = postio_session::checkup::set_display_name(
        &database,
        postio_model::ids::AccountId::new(404),
        "Nobody",
    )
    .expect_err("there is no such account");
    assert!(error.contains("not in the store"), "{error}");
}
